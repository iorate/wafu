use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use zip::read::ZipFile;
use zip::result::ZipError;
use zip::write::{FileOptions, SimpleFileOptions};
use zip::{AesMode, CompressionMethod, ExtraField, ZipArchive, ZipWriter};

use super::names::{decode_name, split_name};
use super::{ArchiveEntry, ArchiveEntryKind, ArchiveError, ArchiveReader, ArchiveWriter};

const ZIP64_THRESHOLD: u64 = 4 * 1024 * 1024 * 1024;

pub(crate) struct ZipReader {
    archive: ZipArchive<File>,
    entries: Vec<ArchiveEntry>,
}

pub(crate) struct ZipArchiveWriter {
    writer: ZipWriter<File>,
    password: Option<String>,
}

impl ZipReader {
    pub(crate) fn open(path: &Path) -> Result<Self, ArchiveError> {
        let file = File::open(path)?;
        let mut archive = ZipArchive::new(file).map_err(convert)?;
        let mut entries = Vec::with_capacity(archive.len());
        for index in 0..archive.len() {
            let file = archive.by_index_raw(index).map_err(convert)?;
            entries.push(entry_of(&file));
        }
        Ok(Self { archive, entries })
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
        let file = match password {
            Some(password) => self.archive.by_index_decrypt(index, password.as_bytes()),
            None => self.archive.by_index(index),
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

impl Write for ZipArchiveWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
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

    fn start_file(
        &mut self,
        path: &str,
        size: u64,
        modified: Option<SystemTime>,
        mode: Option<u32>,
    ) -> io::Result<()> {
        let options = options(self.password.as_deref(), size, modified, mode);
        self.writer.start_file(path, options).map_err(io_error)
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
    let kind = if name.ends_with(['/', '\\']) {
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
        mode: file.unix_mode(),
        encrypted: file.encrypted(),
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
    use std::fs;

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
                .start_file("d/f.txt", 5, Some(modified), Some(0o600))
                .unwrap();
            writer.write_all(b"hello").unwrap();
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
        assert!(!entries[1].encrypted);

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
    fn encrypted_entries_need_the_right_password() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("p.zip");
        {
            let mut writer = Box::new(ZipArchiveWriter::new(
                File::create(&path).unwrap(),
                Some("secret"),
            ));
            writer.start_file("f.txt", 2, None, None).unwrap();
            writer.write_all(b"hi").unwrap();
            writer.finish().unwrap();
        }

        let mut reader = ZipReader::open(&path).unwrap();

        assert!(reader.entries()[0].encrypted);
        assert!(matches!(
            reader.open(0, None),
            Err(ArchiveError::PasswordRequired)
        ));
        assert!(matches!(
            reader.open(0, Some("wrong")),
            Err(ArchiveError::WrongPassword)
        ));
        let mut content = String::new();
        reader
            .open(0, Some("secret"))
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "hi");
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_invalid() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("x.zip");
        fs::write(&path, b"not a zip").unwrap();

        assert!(matches!(
            ZipReader::open(&path),
            Err(ArchiveError::Invalid(_))
        ));
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
        fs::write(&path, raw_zip(&raw)).unwrap();

        let reader = ZipReader::open(&path).unwrap();

        assert_eq!(reader.entries()[0].kind, ArchiveEntryKind::File);
        assert_eq!(reader.entries()[0].components, ["表"]);
    }

    #[test]
    fn dos_times_round_trip_to_even_seconds() {
        let odd = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_001);

        let back = system_time_of(dos_time_of(odd).unwrap()).unwrap();

        assert_eq!(back, odd - Duration::from_secs(1));
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
