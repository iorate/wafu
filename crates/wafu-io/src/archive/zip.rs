use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
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

pub(crate) struct ZipReader {
    archive: ZipArchive<File>,
    entries: Vec<ArchiveEntry>,
    // Where each entry is in the crate's index, which also holds the root and
    // the entries replaced by later ones.
    positions: Vec<usize>,
}

pub(crate) struct ZipArchiveWriter {
    writer: ZipWriter<File>,
    password: Option<String>,
}

impl ZipReader {
    pub(crate) fn open(path: &Path) -> Result<Self, ArchiveError> {
        let file = File::open(path)?;
        let mut archive = ZipArchive::new(file).map_err(convert)?;
        let mut found = Vec::with_capacity(archive.len());
        let mut latest: HashMap<Vec<String>, usize> = HashMap::new();
        let mut replaced = Vec::with_capacity(archive.len());
        for position in 0..archive.len() {
            let file = archive.by_index_raw(position).map_err(convert)?;
            let entry = entry_of(&file);
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
        let file = match password {
            Some(password) => self.archive.by_index_decrypt(position, password.as_bytes()),
            None => self.archive.by_index(position),
        }
        .map_err(convert)?;
        Ok(Box::new(file))
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
