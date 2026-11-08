use std::fs::File;
use std::io::{self, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bzip2::read::MultiBzDecoder;
use bzip2::write::BzEncoder;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use lzma_rust2::{XzOptions, XzReader, XzWriter};
use tar::{Archive, Builder, Entries, Entry, EntryType, Header};

use super::names::{decode_name, split_name};
use super::{ArchiveEntry, ArchiveEntryKind, ArchiveError, ArchiveReader, ArchiveWriter};

const DEFAULT_FILE_MODE: u32 = 0o644;
const DEFAULT_DIR_MODE: u32 = 0o755;
const LINK_MODE: u32 = 0o777;
const XZ_PRESET: u32 = 6;
const SPARSE: &str = "sparse entries are not supported";
const MULTI_VOLUME: &str = "multi-volume archives are not supported";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
}

// A tar can only be read front to back, so the whole archive is scanned once
// when opened, and entries are then read from a stream that only moves
// forward and starts over when asked for something behind it. Reading the
// entries in index order, as an extraction does, costs one more pass.
pub(crate) struct TarReader {
    path: PathBuf,
    compression: Compression,
    entries: Vec<ArchiveEntry>,
    contents: Vec<Content>,
    stream: Option<Stream>,
}

pub(crate) struct TarWriter {
    builder: Builder<Encoder>,
}

// Where the content of an entry is.
#[derive(Debug, Clone)]
enum Content {
    // `len` bytes from `start` of the uncompressed stream.
    Span { start: u64, len: u64 },
    // A link's content is its target, which the header holds.
    Target(String),
    // A hard link has the content of the earlier entry at this index.
    HardLink(usize),
    // A hard link to a name no earlier entry has.
    Dangling,
    // Content wafu cannot read, with the reason: one spread over holes the
    // stream positions do not describe, or a fragment of a multi-volume file.
    Unsupported(&'static str),
}

struct Stream {
    source: Source,
    pos: u64,
}

enum Source {
    Plain(BufReader<File>),
    Decoded(Box<dyn Read>),
}

// Reads one entry from the stream, no further than its end, and leaves the
// stream positioned after what was read.
struct EntryReader<'a> {
    stream: &'a mut Stream,
    remaining: u64,
}

// What an entry's PAX records say beyond what the crate applies itself.
#[derive(Default)]
struct PaxRecords {
    sparse: bool,
    sparse_name: Option<String>,
    sparse_size: Option<u64>,
    mtime: Option<String>,
}

struct Counted<R> {
    inner: R,
    read: u64,
}

enum Encoder {
    Plain(BufWriter<File>),
    Gzip(GzEncoder<BufWriter<File>>),
    Bzip2(BzEncoder<BufWriter<File>>),
    Xz(XzWriter<BufWriter<File>>),
}

impl TarReader {
    pub(crate) fn open(path: &Path, compression: Compression) -> Result<Self, ArchiveError> {
        let file = BufReader::new(File::open(path)?);
        // A plain tar can seek past the content; a decoded one reads through it.
        let (entries, contents) = if compression == Compression::None {
            let mut archive = Archive::new(file);
            scan(archive.entries_with_seek()?)?
        } else {
            let mut archive = Archive::new(decoder(compression, file));
            scan(archive.entries()?)?
        };
        Ok(Self {
            path: path.to_path_buf(),
            compression,
            entries,
            contents,
            stream: None,
        })
    }
}

impl ArchiveReader for TarReader {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    fn open(
        &mut self,
        index: usize,
        _password: Option<&str>,
    ) -> Result<Box<dyn Read + '_>, ArchiveError> {
        let content = match self.contents[index] {
            Content::HardLink(target) => self.contents[target].clone(),
            ref content => content.clone(),
        };
        match content {
            Content::Target(target) => Ok(Box::new(Cursor::new(target.into_bytes()))),
            Content::Unsupported(reason) => Err(ArchiveError::Invalid(reason.to_owned())),
            Content::HardLink(_) | Content::Dangling => Err(ArchiveError::Invalid(
                "the target of the hard link is not in the archive".to_owned(),
            )),
            Content::Span { start, len } => {
                if self.stream.as_ref().is_none_or(|stream| stream.pos > start) {
                    self.stream = Some(Stream::open(&self.path, self.compression)?);
                }
                let stream = self.stream.as_mut().expect("the stream was just opened");
                stream.skip_to(start)?;
                Ok(Box::new(EntryReader {
                    stream,
                    remaining: len,
                }))
            }
        }
    }

    fn hard_link_target(&self, index: usize) -> Option<usize> {
        match self.contents[index] {
            Content::HardLink(target) => Some(target),
            _ => None,
        }
    }
}

impl TarWriter {
    pub(crate) fn new(file: File, compression: Compression) -> io::Result<Self> {
        Ok(Self {
            builder: Builder::new(Encoder::new(file, compression)?),
        })
    }
}

impl ArchiveWriter for TarWriter {
    fn add_dir(
        &mut self,
        path: &str,
        modified: Option<SystemTime>,
        mode: Option<u32>,
    ) -> io::Result<()> {
        let mut header = header(
            EntryType::Directory,
            0,
            modified,
            mode.unwrap_or(DEFAULT_DIR_MODE),
        );
        self.builder
            .append_data(&mut header, format!("{path}/"), io::empty())
    }

    fn add_file(
        &mut self,
        path: &str,
        size: u64,
        modified: Option<SystemTime>,
        mode: Option<u32>,
        data: &mut dyn Read,
    ) -> io::Result<()> {
        let mut header = header(
            EntryType::Regular,
            size,
            modified,
            mode.unwrap_or(DEFAULT_FILE_MODE),
        );
        // The header holds `size` and the crate pads to what it copies, so
        // a file that changes in between would throw the headers after it
        // off. Such a file fails the archive instead.
        let mut counted = Counted {
            inner: (&mut *data).take(size),
            read: 0,
        };
        self.builder.append_data(&mut header, path, &mut counted)?;
        if counted.read != size || data.read(&mut [0])? != 0 {
            return Err(io::Error::other(
                "the file changed while it was being added",
            ));
        }
        Ok(())
    }

    fn add_symlink(
        &mut self,
        path: &str,
        target: &str,
        modified: Option<SystemTime>,
    ) -> io::Result<()> {
        let mut header = header(EntryType::Symlink, 0, modified, LINK_MODE);
        self.builder.append_link(&mut header, path, target)
    }

    fn finish(self: Box<Self>) -> io::Result<()> {
        self.builder.into_inner()?.finish()
    }
}

impl Stream {
    fn open(path: &Path, compression: Compression) -> io::Result<Self> {
        let file = BufReader::new(File::open(path)?);
        let source = match compression {
            Compression::None => Source::Plain(file),
            _ => Source::Decoded(decoder(compression, file)),
        };
        Ok(Self { source, pos: 0 })
    }

    // Moves forward to `target`, which is never behind the position.
    fn skip_to(&mut self, target: u64) -> io::Result<()> {
        match &mut self.source {
            Source::Plain(file) => {
                file.seek(SeekFrom::Start(target))?;
            }
            Source::Decoded(reader) => {
                io::copy(
                    &mut reader.by_ref().take(target - self.pos),
                    &mut io::sink(),
                )?;
            }
        }
        self.pos = target;
        Ok(())
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = match &mut self.source {
            Source::Plain(file) => file.read(buf)?,
            Source::Decoded(reader) => reader.read(buf)?,
        };
        self.pos += read as u64;
        Ok(read)
    }
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.read += read as u64;
        Ok(read)
    }
}

impl Read for EntryReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 || buf.is_empty() {
            return Ok(0);
        }
        let want =
            usize::try_from(self.remaining).map_or(buf.len(), |remaining| remaining.min(buf.len()));
        let read = self.stream.read(&mut buf[..want])?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the archive ends inside an entry",
            ));
        }
        self.remaining -= read as u64;
        Ok(read)
    }
}

impl Encoder {
    fn new(file: File, compression: Compression) -> io::Result<Self> {
        let file = BufWriter::new(file);
        Ok(match compression {
            Compression::None => Self::Plain(file),
            Compression::Gzip => Self::Gzip(GzEncoder::new(file, flate2::Compression::default())),
            Compression::Bzip2 => Self::Bzip2(BzEncoder::new(file, bzip2::Compression::default())),
            Compression::Xz => Self::Xz(XzWriter::new(file, XzOptions::with_preset(XZ_PRESET))?),
        })
    }

    fn finish(self) -> io::Result<()> {
        let mut file = match self {
            Self::Plain(file) => file,
            Self::Gzip(encoder) => encoder.finish()?,
            Self::Bzip2(encoder) => encoder.finish()?,
            Self::Xz(encoder) => encoder.finish()?,
        };
        file.flush()
    }
}

impl Write for Encoder {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(file) => file.write(buf),
            Self::Gzip(encoder) => encoder.write(buf),
            Self::Bzip2(encoder) => encoder.write(buf),
            Self::Xz(encoder) => encoder.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(file) => file.flush(),
            Self::Gzip(encoder) => encoder.flush(),
            Self::Bzip2(encoder) => encoder.flush(),
            Self::Xz(encoder) => encoder.flush(),
        }
    }
}

// The index of the archive, in one pass. The crate resolves the GNU and PAX
// extensions, so long names arrive whole and their carrier entries do not
// show up.
fn scan<R: Read>(entries: Entries<'_, R>) -> io::Result<(Vec<ArchiveEntry>, Vec<Content>)> {
    let mut listed: Vec<ArchiveEntry> = Vec::new();
    let mut contents: Vec<Content> = Vec::new();
    for entry in entries {
        let mut entry = entry?;
        let pax = pax_records(&mut entry)?;
        let header = entry.header();
        let name = pax
            .sparse_name
            .clone()
            .unwrap_or_else(|| decode_name(&entry.path_bytes()));
        let components = split_name(&name);
        // `./`, which `tar -C dir -cf x.tar .` starts with, and an empty name
        // are the root, which has no entry of its own.
        if components.is_empty() {
            continue;
        }
        let mode = header.mode().ok().map(|mode| mode & 0o7777);
        // A PAX record holds what the header field cannot: a time before
        // 1970 or past 2242, or a fraction of a second. GNU tar writes a time
        // before 1970 in base-256 two's complement, which decodes to a huge
        // number. Either way a time before 1970 is left out.
        let modified = match &pax.mtime {
            Some(record) => pax_time(record),
            None => header
                .mtime()
                .ok()
                .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds))),
        };
        let Some((kind, size, content)) = member(&entry, &name, &pax, &listed, &contents) else {
            continue;
        };
        listed.push(ArchiveEntry {
            components,
            kind,
            size,
            modified,
            mode,
        });
        contents.push(content);
    }
    Ok((listed, contents))
}

// What an entry is, how large, and where its content is; `None` for one
// that is not a member.
fn member<R: Read>(
    entry: &Entry<'_, R>,
    name: &str,
    pax: &PaxRecords,
    listed: &[ArchiveEntry],
    contents: &[Content],
) -> Option<(ArchiveEntryKind, u64, Content)> {
    let link_name = || {
        entry
            .link_name_bytes()
            .map(|bytes| decode_name(&bytes))
            .unwrap_or_default()
    };
    Some(match entry.header().entry_type() {
        // A PAX sparse file is a regular one holding the map of its
        // data, under a made-up name.
        _ if pax.sparse => (
            ArchiveEntryKind::File,
            pax.sparse_size.unwrap_or(entry.size()),
            Content::Unsupported(SPARSE),
        ),
        // The old formats have no directory type and mark a directory by
        // a slash at the end of its name, as GNU tar still reads them. A
        // GNU incremental dump lists a directory's contents as its data.
        kind if kind == EntryType::Directory
            || kind.as_byte() == b'D'
            || (matches!(kind, EntryType::Regular | EntryType::Continuous)
                && name.ends_with('/')) =>
        {
            (
                ArchiveEntryKind::Dir,
                0,
                Content::Span {
                    start: entry.raw_file_position(),
                    len: 0,
                },
            )
        }
        EntryType::Symlink => {
            let target = link_name();
            (
                ArchiveEntryKind::Symlink,
                target.len() as u64,
                Content::Target(target),
            )
        }
        // A hard link shares the content of an entry that came before it,
        // and a link to a link shares that of the first.
        EntryType::Link => {
            let target = split_name(&link_name());
            let found = listed.iter().rposition(|listed| {
                listed.kind == ArchiveEntryKind::File && listed.components == target
            });
            match found {
                Some(index) => (
                    ArchiveEntryKind::File,
                    listed[index].size,
                    match contents[index] {
                        Content::HardLink(first) => Content::HardLink(first),
                        Content::Dangling => Content::Dangling,
                        _ => Content::HardLink(index),
                    },
                ),
                None => (ArchiveEntryKind::File, 0, Content::Dangling),
            }
        }
        EntryType::GNUSparse => (
            ArchiveEntryKind::File,
            entry.size(),
            Content::Unsupported(SPARSE),
        ),
        // A continuation holds the rest of a file begun on the volume
        // before, which this archive does not have.
        kind if kind.as_byte() == b'M' => (
            ArchiveEntryKind::File,
            entry.size(),
            Content::Unsupported(MULTI_VOLUME),
        ),
        // Nothing could be made of devices on extraction, and a whole
        // archive should not fail over one. The crate resolves the
        // extensions but still hands out a PAX global header (what
        // `git archive` starts with), which is not a member either, and
        // neither is a GNU volume label.
        EntryType::Char
        | EntryType::Block
        | EntryType::Fifo
        | EntryType::XGlobalHeader
        | EntryType::XHeader
        | EntryType::GNULongName
        | EntryType::GNULongLink => return None,
        kind if kind.as_byte() == b'V' => return None,
        // Unknown types are files, as the format says.
        _ => (
            ArchiveEntryKind::File,
            entry.size(),
            Content::Span {
                start: entry.raw_file_position(),
                len: entry.size(),
            },
        ),
    })
}

fn pax_records<R: Read>(entry: &mut Entry<'_, R>) -> io::Result<PaxRecords> {
    let mut records = PaxRecords::default();
    // A global header's records are its content and not about the header,
    // and they are left out with it.
    let kind = entry.header().entry_type();
    if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
        return Ok(records);
    }
    let Some(extensions) = entry.pax_extensions()? else {
        return Ok(records);
    };
    for extension in extensions {
        let extension = extension?;
        match extension.key() {
            Ok("GNU.sparse.name") => {
                records.sparse = true;
                records.sparse_name = Some(decode_name(extension.value_bytes()));
            }
            Ok("GNU.sparse.realsize" | "GNU.sparse.size") => {
                records.sparse = true;
                records.sparse_size = extension.value().ok().and_then(|size| size.parse().ok());
            }
            Ok(key) if key.starts_with("GNU.sparse.") => records.sparse = true,
            Ok("mtime") => {
                records.mtime = Some(String::from_utf8_lossy(extension.value_bytes()).into_owned());
            }
            _ => {}
        }
    }
    Ok(records)
}

// Decimal seconds with an optional fraction, which is dropped.
fn pax_time(record: &str) -> Option<SystemTime> {
    let whole = record.split_once('.').map_or(record, |(whole, _)| whole);
    let seconds: u64 = whole.parse().ok()?;
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
}

// Multi-member streams are what parallel compressors (pigz, pbzip2) write.
fn decoder(compression: Compression, file: BufReader<File>) -> Box<dyn Read> {
    match compression {
        Compression::None => Box::new(file),
        Compression::Gzip => Box::new(MultiGzDecoder::new(file)),
        Compression::Bzip2 => Box::new(MultiBzDecoder::new(file)),
        Compression::Xz => Box::new(XzReader::new(file, true)),
    }
}

// Owner fields are left at zero; wafu does not record who made the archive.
fn header(kind: EntryType, size: u64, modified: Option<SystemTime>, mode: u32) -> Header {
    let mut header = Header::new_gnu();
    header.set_entry_type(kind);
    header.set_size(size);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    let mtime = modified
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs());
    header.set_mtime(mtime);
    header
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    const ALL: [Compression; 4] = [
        Compression::None,
        Compression::Gzip,
        Compression::Bzip2,
        Compression::Xz,
    ];

    #[test]
    fn reads_what_it_wrote_in_every_compression() {
        let dir = TempDir::new().unwrap();
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let long = "x".repeat(150);
        for compression in ALL {
            let path = dir.path().join("a.tar");
            {
                let mut writer =
                    Box::new(TarWriter::new(File::create(&path).unwrap(), compression).unwrap());
                writer.add_dir("d", Some(modified), Some(0o755)).unwrap();
                writer
                    .add_file(
                        "d/f.txt",
                        5,
                        Some(modified),
                        Some(0o600),
                        &mut &b"hello"[..],
                    )
                    .unwrap();
                writer
                    .add_file(&format!("d/{long}"), 4, None, None, &mut &b"long"[..])
                    .unwrap();
                writer.add_symlink("l", "d/f.txt", Some(modified)).unwrap();
                writer.finish().unwrap();
            }

            let mut reader = TarReader::open(&path, compression).unwrap();

            let entries = reader.entries().to_vec();
            assert_eq!(entries.len(), 4, "{compression:?}");
            assert_eq!(entries[0].components, ["d"]);
            assert_eq!(entries[0].kind, ArchiveEntryKind::Dir);
            assert_eq!(entries[0].mode, Some(0o755));
            assert_eq!(entries[1].components, ["d", "f.txt"]);
            assert_eq!(entries[1].kind, ArchiveEntryKind::File);
            assert_eq!(entries[1].size, 5);
            assert_eq!(entries[1].mode, Some(0o600));
            assert_eq!(entries[1].modified, Some(modified));
            assert_eq!(entries[2].components, ["d", long.as_str()]);
            assert_eq!(entries[2].mode, Some(DEFAULT_FILE_MODE));
            assert_eq!(entries[3].kind, ArchiveEntryKind::Symlink);
            assert_eq!(entries[3].size, 7);
            assert_eq!(content(&mut reader, 1), b"hello");
            assert_eq!(content(&mut reader, 2), b"long");
            assert_eq!(content(&mut reader, 3), b"d/f.txt");
        }
    }

    #[test]
    fn going_back_reopens_the_stream_and_a_partial_read_does_not_break_the_next() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.tgz");
        {
            let mut writer =
                Box::new(TarWriter::new(File::create(&path).unwrap(), Compression::Gzip).unwrap());
            writer
                .add_file("a", 6, None, None, &mut &b"first_"[..])
                .unwrap();
            writer
                .add_file("b", 6, None, None, &mut &b"second"[..])
                .unwrap();
            writer
                .add_file("c", 5, None, None, &mut &b"third"[..])
                .unwrap();
            writer.finish().unwrap();
        }
        let mut reader = TarReader::open(&path, Compression::Gzip).unwrap();

        let mut head = [0; 2];
        reader.open(1, None).unwrap().read_exact(&mut head).unwrap();
        assert_eq!(&head, b"se");
        assert_eq!(content(&mut reader, 2), b"third");
        assert_eq!(content(&mut reader, 0), b"first_");
        assert_eq!(content(&mut reader, 1), b"second");
    }

    #[test]
    fn a_hard_link_reads_the_content_of_its_target() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.tar");
        {
            let mut builder = Builder::new(File::create(&path).unwrap());
            let mut orig = header(EntryType::Regular, 4, None, 0o644);
            builder
                .append_data(&mut orig, "orig", &b"data"[..])
                .unwrap();
            let mut same = header_of(EntryType::Link);
            builder.append_link(&mut same, "same", "orig").unwrap();
            let mut lost = header_of(EntryType::Link);
            builder.append_link(&mut lost, "lost", "gone").unwrap();
            builder.finish().unwrap();
        }

        let mut reader = TarReader::open(&path, Compression::None).unwrap();

        assert_eq!(reader.entries()[1].kind, ArchiveEntryKind::File);
        assert_eq!(reader.entries()[1].size, 4);
        assert_eq!(content(&mut reader, 1), b"data");
        assert_eq!(reader.entries()[2].size, 0);
        assert!(matches!(
            reader.open(2, None),
            Err(ArchiveError::Invalid(message)) if message.contains("hard link")
        ));
    }

    #[test]
    fn devices_are_left_out_and_sparse_files_cannot_be_read() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.tar");
        {
            let mut builder = Builder::new(File::create(&path).unwrap());
            let mut fifo = header_of(EntryType::Fifo);
            builder.append_data(&mut fifo, "pipe", io::empty()).unwrap();
            let mut file = header(EntryType::Regular, 1, None, 0o644);
            builder.append_data(&mut file, "f", &b"x"[..]).unwrap();
            builder.finish().unwrap();
        }
        let reader = TarReader::open(&path, Compression::None).unwrap();
        assert_eq!(reader.entries().len(), 1);
        assert_eq!(reader.entries()[0].components, ["f"]);

        let mut reader = TarReader {
            path,
            compression: Compression::None,
            entries: vec![reader.entries()[0].clone()],
            contents: vec![Content::Unsupported(SPARSE)],
            stream: None,
        };
        assert!(matches!(
            reader.open(0, None),
            Err(ArchiveError::Invalid(message)) if message.contains("sparse")
        ));
    }

    #[test]
    fn a_pax_sparse_file_is_listed_under_its_name_and_cannot_be_read() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.tar");
        {
            let mut builder = Builder::new(File::create(&path).unwrap());
            builder
                .append_pax_extensions([
                    ("GNU.sparse.major", &b"1"[..]),
                    ("GNU.sparse.minor", b"0"),
                    ("GNU.sparse.name", b"d/holes.bin"),
                    ("GNU.sparse.realsize", b"1048576"),
                ])
                .unwrap();
            let map = b"1\n0\n5\n";
            let mut stored = header(EntryType::Regular, map.len() as u64, None, 0o644);
            builder
                .append_data(&mut stored, "d/GNUSparseFile.0/holes.bin", &map[..])
                .unwrap();
            builder.finish().unwrap();
        }

        let mut reader = TarReader::open(&path, Compression::None).unwrap();

        assert_eq!(reader.entries().len(), 1);
        assert_eq!(reader.entries()[0].components, ["d", "holes.bin"]);
        assert_eq!(reader.entries()[0].size, 1_048_576);
        assert!(matches!(
            reader.open(0, None),
            Err(ArchiveError::Invalid(message)) if message.contains("sparse")
        ));
    }

    #[test]
    fn a_volume_label_is_left_out_a_dumpdir_is_a_directory_and_a_continuation_cannot_be_read() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.tar");
        {
            let mut builder = Builder::new(File::create(&path).unwrap());
            let mut label = header_of(EntryType::new(b'V'));
            builder
                .append_data(&mut label, "backup", io::empty())
                .unwrap();
            let listing = b"Yx\0\0";
            let mut dump = header(EntryType::new(b'D'), listing.len() as u64, None, 0o755);
            builder.append_data(&mut dump, "d", &listing[..]).unwrap();
            let mut rest = header(EntryType::new(b'M'), 1, None, 0o644);
            builder.append_data(&mut rest, "big", &b"x"[..]).unwrap();
            builder.finish().unwrap();
        }

        let mut reader = TarReader::open(&path, Compression::None).unwrap();

        let entries = reader.entries().to_vec();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].components, ["d"]);
        assert_eq!(entries[0].kind, ArchiveEntryKind::Dir);
        assert_eq!(entries[0].size, 0);
        assert_eq!(entries[1].components, ["big"]);
        assert!(matches!(
            reader.open(1, None),
            Err(ArchiveError::Invalid(message)) if message.contains("multi-volume")
        ));
    }

    #[test]
    fn a_time_before_1970_and_a_pax_global_header_are_left_out() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.tar");
        {
            let mut builder = Builder::new(File::create(&path).unwrap());
            let mut global = header_of(EntryType::XGlobalHeader);
            global.set_size(11);
            builder
                .append_data(&mut global, "pax_global_header", &b"52 comment=x"[..11])
                .unwrap();
            let mut old = header(EntryType::Regular, 1, None, 0o644);
            // What GNU tar writes for a negative time, read as unsigned.
            old.set_mtime(u64::MAX - 1000);
            builder.append_data(&mut old, "old", &b"x"[..]).unwrap();
            builder.finish().unwrap();
        }

        let mut reader = TarReader::open(&path, Compression::None).unwrap();

        assert_eq!(reader.entries().len(), 1);
        assert_eq!(reader.entries()[0].components, ["old"]);
        assert_eq!(reader.entries()[0].modified, None);
        assert_eq!(content(&mut reader, 0), b"x");
    }

    #[test]
    fn a_pax_time_is_taken_over_the_header_to_the_second() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.tar");
        let records: [&[u8]; 3] = [b"10413792001", b"-1000", b"1700000000.75"];
        {
            let mut builder = Builder::new(File::create(&path).unwrap());
            for (index, record) in records.iter().enumerate() {
                builder.append_pax_extensions([("mtime", *record)]).unwrap();
                let mut file = header(EntryType::Regular, 0, None, 0o644);
                file.set_mtime(1);
                builder
                    .append_data(&mut file, index.to_string(), io::empty())
                    .unwrap();
            }
            builder.finish().unwrap();
        }

        let reader = TarReader::open(&path, Compression::None).unwrap();

        let modified: Vec<Option<SystemTime>> = reader
            .entries()
            .iter()
            .map(|entry| entry.modified)
            .collect();
        assert_eq!(
            modified,
            [
                Some(UNIX_EPOCH + Duration::from_secs(10_413_792_001)),
                None,
                Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            ]
        );
    }

    #[test]
    fn a_file_whose_size_changed_fails_instead_of_misaligning_the_archive() {
        let dir = TempDir::new().unwrap();
        for (size, data) in [(5, &b"abc"[..]), (3, &b"abcde"[..])] {
            let path = dir.path().join("a.tar");
            let mut writer =
                TarWriter::new(File::create(&path).unwrap(), Compression::None).unwrap();

            let error = writer
                .add_file("f", size, None, None, &mut &data[..])
                .unwrap_err();

            assert!(error.to_string().contains("changed"), "{size}");
        }
    }

    #[test]
    fn a_compressed_file_that_is_not_a_tar_is_an_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.gz");
        let mut encoder = GzEncoder::new(
            BufWriter::new(File::create(&path).unwrap()),
            flate2::Compression::default(),
        );
        encoder.write_all(b"just some text").unwrap();
        encoder.finish().unwrap().flush().unwrap();

        assert!(TarReader::open(&path, Compression::Gzip).is_err());
    }

    #[test]
    fn a_truncated_entry_is_an_error_rather_than_a_short_read() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.tar");
        {
            let mut writer =
                Box::new(TarWriter::new(File::create(&path).unwrap(), Compression::None).unwrap());
            writer
                .add_file("a", 4, None, None, &mut &b"data"[..])
                .unwrap();
            writer.finish().unwrap();
        }
        let mut reader = TarReader::open(&path, Compression::None).unwrap();
        // Everything after the header: the content and the end blocks.
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(512 + 2)
            .unwrap();

        let mut content = Vec::new();
        let error = reader
            .open(0, None)
            .unwrap()
            .read_to_end(&mut content)
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    fn content(reader: &mut TarReader, index: usize) -> Vec<u8> {
        let mut content = Vec::new();
        reader
            .open(index, None)
            .unwrap()
            .read_to_end(&mut content)
            .unwrap();
        content
    }

    fn header_of(kind: EntryType) -> Header {
        header(kind, 0, None, 0o644)
    }
}
