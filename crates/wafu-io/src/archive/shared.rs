use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;

// Reads a file at a position of its own rather than the handle's, so that
// any number of readers share one handle without moving each other. An
// archive read again from its start is read through the handle it was
// indexed from, not its path, which may be another file by then.
pub(crate) struct SharedFile {
    file: Arc<File>,
    pos: u64,
}

impl SharedFile {
    pub(crate) fn new(file: File) -> Self {
        Self {
            file: Arc::new(file),
            pos: 0,
        }
    }

    pub(crate) fn rewound(&self) -> Self {
        Self {
            file: Arc::clone(&self.file),
            pos: 0,
        }
    }
}

impl Read for SharedFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = read_at(&self.file, buf, self.pos)?;
        self.pos += read as u64;
        Ok(read)
    }
}

impl Seek for SharedFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(offset) => self.pos.checked_add_signed(offset),
            SeekFrom::End(offset) => self.file.metadata()?.len().checked_add_signed(offset),
        };
        self.pos = target.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek to a negative or overflowing position",
            )
        })?;
        Ok(self.pos)
    }
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

// This moves the handle's position, which nothing here reads.
#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn readers_of_one_file_keep_their_own_positions() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a");
        fs::write(&path, b"0123456789").unwrap();
        let mut first = SharedFile::new(File::open(&path).unwrap());
        let mut second = first.rewound();
        let mut buf = [0; 3];

        first.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"012");
        assert_eq!(second.seek(SeekFrom::End(-3)).unwrap(), 7);
        second.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"789");
        first.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"345");
        assert_eq!(first.rewound().read(&mut buf).unwrap(), 3);
        assert_eq!(&buf, b"012");
    }

    #[test]
    fn seeking_before_the_start_fails() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a");
        fs::write(&path, b"0123").unwrap();
        let mut file = SharedFile::new(File::open(&path).unwrap());

        let error = file.seek(SeekFrom::End(-5)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(file.stream_position().unwrap(), 0);
    }
}
