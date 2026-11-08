use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use zip::read::ZipFile;
use zip::result::ZipError;
use zip::write::{FileOptions, SimpleFileOptions};
use zip::{AesMode, CompressionMethod, ExtraField, System, ZipArchive, ZipWriter};

use super::names::{decode_name, split_name};
use super::{ArchiveEntry, ArchiveEntryKind, ArchiveError, ArchiveReader, ArchiveWriter};

const ZIP64_THRESHOLD: u64 = 4 * 1024 * 1024 * 1024;
const FILE_TYPE: u32 = 0o170_000;
const DIR_TYPE: u32 = 0o040_000;
const LOCAL_SIGNATURE: &[u8] = b"PK\x03\x04";
const END_SIGNATURE: &[u8] = b"PK\x05\x06";
// The end record is at most its fixed part and the longest comment away from
// the end of the file.
const END_SEARCH_LEN: u64 = 22 + 0xffff;
pub(crate) const NOT_ZIP: &str = "not a zip archive";
const DAMAGED: &str = "the zip archive is damaged";

pub(crate) struct ZipReader {
    archive: ZipArchive<File>,
    entries: Vec<ArchiveEntry>,
    // Where each entry is in the crate's index, which also holds the root and
    // the entries replaced by later ones.
    positions: Vec<usize>,
}

// Tells which entry a read failed in, rather than what the crate found
// wrong, which is about the format.
struct EntryReader<R> {
    file: R,
    name: String,
}

pub(crate) struct ZipArchiveWriter {
    writer: ZipWriter<File>,
    password: Option<String>,
}

impl ZipReader {
    pub(crate) fn open(path: &Path) -> Result<Self, ArchiveError> {
        let file = File::open(path)?;
        let mut archive = match ZipArchive::new(file) {
            Ok(archive) => archive,
            Err(error) if is_damage(&error) => {
                let message = if has_signature(path)? {
                    DAMAGED
                } else {
                    NOT_ZIP
                };
                return Err(ArchiveError::Invalid(message.to_owned()));
            }
            Err(error) => return Err(convert(error)),
        };
        let mut found = Vec::with_capacity(archive.len());
        let mut latest: HashMap<Vec<String>, usize> = HashMap::new();
        let mut replaced = Vec::with_capacity(archive.len());
        for position in 0..archive.len() {
            let read = archive.by_index_raw(position).map(|file| entry_of(&file));
            let entry = match read {
                Ok(entry) => entry,
                Err(error) if is_damage(&error) => {
                    let name = archive
                        .by_index_data(position)
                        .map(|entry| decode_name(entry.name_raw()))
                        .unwrap_or_default();
                    return Err(ArchiveError::Invalid(format!("{DAMAGED} in {name}")));
                }
                Err(error) => return Err(convert(error)),
            };
            // `./` and `/`, which an archive made from `.` may start with, and
            // an empty name are the root, which has no entry of its own.
            if entry.components.is_empty() {
                continue;
            }
            // The crate merges entries only when their names are spelled
            // alike, so `a.txt` and `./a.txt` both reach here. The later one
            // replaces the earlier, as it does for names spelled alike.
            if let Some(earlier) = latest.insert(entry.components.clone(), found.len()) {
                replaced[earlier] = true;
            }
            replaced.push(false);
            found.push((entry, position));
        }
        let (entries, positions) = found
            .into_iter()
            .zip(replaced)
            .filter(|(_, replaced)| !replaced)
            .map(|(found, _)| found)
            .unzip();
        Ok(Self {
            archive,
            entries,
            positions,
        })
    }
}

impl ArchiveReader for ZipReader {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    fn open(
        &mut self,
        index: usize,
        password: Option<&str>,
    ) -> Result<Box<dyn Read + '_>, ArchiveError> {
        let position = self.positions[index];
        // The crate opens an entry in a method it cannot decompress and fails
        // only when it is read, which would stop an extraction instead of
        // skipping the entry. Inside an AES entry this is the inner method.
        let method = self
            .archive
            .by_index_data(position)
            .map_err(convert)?
            .compression();
        if !matches!(
            method,
            CompressionMethod::Stored
                | CompressionMethod::Deflated
                | CompressionMethod::Deflate64
                | CompressionMethod::Bzip2
        ) {
            return Err(ArchiveError::Invalid(
                "the compression method is not supported".to_owned(),
            ));
        }
        let name = self.entries[index].components.join("/");
        let file = match password {
            Some(password) => self.archive.by_index_decrypt(position, password.as_bytes()),
            None => self.archive.by_index(position),
        }
        .map_err(|error| {
            if is_damage(&error) {
                ArchiveError::Io(damaged_in(&name))
            } else {
                convert(error)
            }
        })?;
        Ok(Box::new(EntryReader { file, name }))
    }
}

impl<R: Read> Read for EntryReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf).map_err(|error| {
            if error.raw_os_error().is_some() || error.kind() == io::ErrorKind::Interrupted {
                error
            } else {
                damaged_in(&self.name)
            }
        })
    }
}

impl ZipArchiveWriter {
    pub(crate) fn new(file: File, password: Option<&str>) -> Self {
        Self {
            writer: ZipWriter::new(file),
            password: password.map(ToOwned::to_owned),
        }
    }
}

impl ArchiveWriter for ZipArchiveWriter {
    fn add_dir(
        &mut self,
        path: &str,
        modified: Option<SystemTime>,
        mode: Option<u32>,
    ) -> io::Result<()> {
        let options = options(self.password.as_deref(), 0, modified, mode);
        self.writer.add_directory(path, options).map_err(io_error)
    }

    fn add_file(
        &mut self,
        path: &str,
        size: u64,
        modified: Option<SystemTime>,
        mode: Option<u32>,
        data: &mut dyn Read,
    ) -> io::Result<()> {
        let options = options(self.password.as_deref(), size, modified, mode);
        self.writer.start_file(path, options).map_err(io_error)?;
        io::copy(data, &mut self.writer)?;
        Ok(())
    }

    fn add_symlink(
        &mut self,
        path: &str,
        target: &str,
        modified: Option<SystemTime>,
    ) -> io::Result<()> {
        let options = options(self.password.as_deref(), 0, modified, None);
        self.writer
            .add_symlink(path, target, options)
            .map_err(io_error)
    }

    fn finish(self: Box<Self>) -> io::Result<()> {
        self.writer.finish().map(drop).map_err(io_error)
    }
}

// The name is decoded here rather than by the crate, whose `name()` reads
// anything that is not UTF-8 as CP437.
fn entry_of(file: &ZipFile<'_, File>) -> ArchiveEntry {
    let name = decode_name(file.name_raw());
    let mode = file.unix_mode();
    // Some tools mark a directory only by its attributes, as Unix mode bits
    // or the DOS directory bit, which the crate reads as the same.
    let kind =
        if name.ends_with(['/', '\\']) || mode.is_some_and(|mode| mode & FILE_TYPE == DIR_TYPE) {
            ArchiveEntryKind::Dir
        } else if file.is_symlink() {
            ArchiveEntryKind::Symlink
        } else {
            ArchiveEntryKind::File
        };
    // The extended timestamp is UTC to the second; the DOS time is local
    // and even-second, so it is the fallback.
    let extended = file.extra_data_fields().find_map(|field| match field {
        ExtraField::ExtendedTimestamp(stamp) => stamp.mod_time(),
        _ => None,
    });
    let modified = extended
        .map(|seconds| SystemTime::UNIX_EPOCH + Duration::from_secs(u64::from(seconds)))
        .or_else(|| file.last_modified().and_then(system_time_of));
    ArchiveEntry {
        components: split_name(&name),
        kind,
        size: file.size(),
        modified,
        // An entry made elsewhere has only DOS attributes, from which the
        // crate makes up a mode; the system's defaults are left to decide.
        mode: mode.filter(|_| file.system() == System::Unix),
    }
}

fn system_time_of(time: zip::DateTime) -> Option<SystemTime> {
    Local
        .with_ymd_and_hms(
            i32::from(time.year()),
            u32::from(time.month()),
            u32::from(time.day()),
            u32::from(time.hour()),
            u32::from(time.minute()),
            u32::from(time.second()),
        )
        .earliest()
        .map(SystemTime::from)
}

fn dos_time_of(time: SystemTime) -> Option<zip::DateTime> {
    let local: DateTime<Local> = time.into();
    zip::DateTime::from_date_and_time(
        u16::try_from(local.year()).ok()?,
        u8::try_from(local.month()).ok()?,
        u8::try_from(local.day()).ok()?,
        u8::try_from(local.hour()).ok()?,
        u8::try_from(local.minute()).ok()?,
        u8::try_from(local.second()).ok()?,
    )
    .ok()
}

fn options(
    password: Option<&str>,
    size: u64,
    modified: Option<SystemTime>,
    mode: Option<u32>,
) -> FileOptions<'_, 'static, ()> {
    let mut options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(size >= ZIP64_THRESHOLD);
    if let Some(time) = modified.and_then(dos_time_of) {
        options = options.last_modified_time(time);
    }
    if let Some(mode) = mode {
        options = options.unix_permissions(mode);
    }
    match password {
        Some(password) => options.with_aes_encryption(AesMode::Aes256, password),
        None => options,
    }
}

// What the crate says of a broken archive, or of one it ran out of, as
// opposed to an error from the system.
fn is_damage(error: &ZipError) -> bool {
    match error {
        ZipError::InvalidArchive(_) => true,
        ZipError::Io(error) => error.raw_os_error().is_none(),
        _ => false,
    }
}

fn damaged_in(name: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{DAMAGED} in {name}"))
}

// The crate finds the end record however far into the file it is, so a
// self-extracting archive with its stub in front opens. A file that starts as
// a zip or has an end record near its end is taken for one that is damaged;
// anything else is not a zip at all.
fn has_signature(path: &Path) -> io::Result<bool> {
    let mut file = File::open(path)?;
    let mut head = Vec::with_capacity(LOCAL_SIGNATURE.len());
    (&mut file)
        .take(LOCAL_SIGNATURE.len() as u64)
        .read_to_end(&mut head)?;
    if head == LOCAL_SIGNATURE || head == END_SIGNATURE {
        return Ok(true);
    }
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(END_SEARCH_LEN)))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    Ok(tail
        .windows(END_SIGNATURE.len())
        .any(|window| window == END_SIGNATURE))
}

fn convert(error: ZipError) -> ArchiveError {
    match error {
        ZipError::UnsupportedArchive(message) if message == ZipError::PASSWORD_REQUIRED => {
            ArchiveError::PasswordRequired
        }
        ZipError::InvalidPassword => ArchiveError::WrongPassword,
        ZipError::Io(error) => ArchiveError::Io(error),
        other => ArchiveError::Invalid(other.to_string()),
    }
}

fn io_error(error: ZipError) -> io::Error {
    match error {
        ZipError::Io(error) => error,
        other => io::Error::other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn reads_what_it_wrote_including_times_and_modes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.zip");
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        {
            let mut writer = Box::new(ZipArchiveWriter::new(File::create(&path).unwrap(), None));
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
            writer.add_symlink("l", "d/f.txt", Some(modified)).unwrap();
            writer.finish().unwrap();
        }

        let mut reader = ZipReader::open(&path).unwrap();

        let entries = reader.entries().to_vec();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].components, ["d"]);
        assert_eq!(entries[0].kind, ArchiveEntryKind::Dir);
        assert_eq!(entries[1].components, ["d", "f.txt"]);
        assert_eq!(entries[1].kind, ArchiveEntryKind::File);
        assert_eq!(entries[1].size, 5);
        // Written on Windows, the entry carries DOS attributes, not a Unix mode.
        if cfg!(unix) {
            assert_eq!(entries[1].mode.map(|mode| mode & 0o777), Some(0o600));
        }
        // DOS times are local and even-second; the written instant is even.
        assert_eq!(entries[1].modified, Some(modified));
        assert_eq!(entries[2].kind, ArchiveEntryKind::Symlink);

        let mut content = String::new();
        reader
            .open(1, None)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "hello");
        let mut target = String::new();
        reader
            .open(2, None)
            .unwrap()
            .read_to_string(&mut target)
            .unwrap();
        assert_eq!(target, "d/f.txt");
    }

    #[test]
    fn entries_marked_as_directories_only_by_their_attributes_are_directories() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        let unix = SimpleFileOptions::default()
            .system(System::Unix)
            .external_attributes(0o040_755 << 16);
        let dos = SimpleFileOptions::default()
            .system(System::Dos)
            .external_attributes(0x10);
        writer.start_file("u", unix).unwrap();
        writer.start_file("w", dos).unwrap();
        writer
            .start_file("f", SimpleFileOptions::default())
            .unwrap();
        writer.finish().unwrap();

        let reader = ZipReader::open(&path).unwrap();

        let kinds: Vec<ArchiveEntryKind> =
            reader.entries().iter().map(|entry| entry.kind).collect();
        assert_eq!(
            kinds,
            [
                ArchiveEntryKind::Dir,
                ArchiveEntryKind::Dir,
                ArchiveEntryKind::File
            ]
        );
    }

    #[test]
    fn only_unix_attributes_bring_a_mode() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        let unix = SimpleFileOptions::default()
            .system(System::Unix)
            .external_attributes(0o100_600 << 16);
        let dos = SimpleFileOptions::default()
            .system(System::Dos)
            .external_attributes(0x01);
        writer.start_file("u", unix).unwrap();
        writer.start_file("w", dos).unwrap();
        writer.finish().unwrap();

        let reader = ZipReader::open(&path).unwrap();

        let modes: Vec<Option<u32>> = reader
            .entries()
            .iter()
            .map(|entry| entry.mode.map(|mode| mode & 0o777))
            .collect();
        assert_eq!(modes, [Some(0o600), None]);
    }

    #[test]
    fn files_that_are_not_zips_are_told_from_damaged_zips() {
        let dir = TempDir::new().unwrap();
        let mut seed = 1u32;
        let noise: Vec<u8> = (0..100)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                seed.to_be_bytes()[1]
            })
            .collect();
        assert!(!noise.windows(2).any(|window| window == b"PK"));
        let zip = small_zip(dir.path());
        let mut stubbed = b"MZ stub ".to_vec();
        stubbed.extend_from_slice(&zip);
        let central = stubbed
            .windows(4)
            .position(|window| window == b"PK\x01\x02")
            .unwrap();
        stubbed[central + 3] = 0;
        let cases: [(&str, Vec<u8>, &str); 4] = [
            ("empty", Vec::new(), NOT_ZIP),
            ("noise", noise, NOT_ZIP),
            ("cut", zip[..zip.len() / 2].to_vec(), DAMAGED),
            ("stubbed", stubbed, DAMAGED),
        ];
        for (name, content, expected) in cases {
            let path = dir.path().join(name);
            std::fs::write(&path, content).unwrap();

            let error = ZipReader::open(&path).err().unwrap();

            assert_eq!(error.to_string(), expected, "{name}");
        }
    }

    #[test]
    fn a_damaged_entry_is_named() {
        let dir = TempDir::new().unwrap();
        let zip = small_zip(dir.path());
        let second = zip
            .windows(4)
            .enumerate()
            .filter(|(_, window)| *window == LOCAL_SIGNATURE)
            .nth(1)
            .unwrap()
            .0;
        let mut header = zip.clone();
        header[second + 3] = 0;
        let header_path = dir.path().join("header.zip");
        std::fs::write(&header_path, header).unwrap();
        let mut content = zip;
        let data = second
            + content[second..]
                .windows(5)
                .position(|window| window == b"hello")
                .unwrap();
        content[data] = b'j';
        let content_path = dir.path().join("content.zip");
        std::fs::write(&content_path, content).unwrap();

        let error = ZipReader::open(&header_path).err().unwrap();
        assert_eq!(error.to_string(), "the zip archive is damaged in b.txt");

        let mut reader = ZipReader::open(&content_path).unwrap();
        let error = reader
            .open(1, None)
            .unwrap()
            .read_to_end(&mut Vec::new())
            .unwrap_err();
        assert_eq!(error.to_string(), "the zip archive is damaged in b.txt");
    }

    #[cfg(windows)]
    #[test]
    fn a_name_whose_last_byte_is_a_backslash_inside_a_character_is_a_file() {
        let raw = [0x95, 0x5c];
        if decode_name(&raw) != "表" {
            return;
        }
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("s.zip");
        std::fs::write(&path, raw_zip(&raw)).unwrap();

        let reader = ZipReader::open(&path).unwrap();

        assert_eq!(reader.entries()[0].kind, ArchiveEntryKind::File);
        assert_eq!(reader.entries()[0].components, ["表"]);
    }

    // Two stored files, `a.txt` and `b.txt`, whose content is `hello`.
    fn small_zip(dir: &Path) -> Vec<u8> {
        let path = dir.join("small.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for name in ["a.txt", "b.txt"] {
            writer.start_file(name, options).unwrap();
            io::Write::write_all(&mut writer, b"hello").unwrap();
        }
        writer.finish().unwrap();
        std::fs::read(path).unwrap()
    }

    #[cfg(windows)]
    fn raw_zip(name: &[u8]) -> Vec<u8> {
        let name_len = u16::try_from(name.len()).unwrap();
        let mut out = Vec::new();
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name);
        let central_offset = u32::try_from(out.len()).unwrap();
        out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(name);
        let central_size = u32::try_from(out.len()).unwrap() - central_offset;
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        out.extend_from_slice(&central_size.to_le_bytes());
        out.extend_from_slice(&central_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }
}
