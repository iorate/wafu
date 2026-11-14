use std::fs::File;
use std::io::{self, Cursor, Read};
use std::path::PathBuf;

use image::{DynamicImage, ImageDecoder, ImageReader, ImageResult};
use wafu_core::{
    Document, ImageData, IoError, OuterArchive, PREVIEW_LIMIT, PreviewSource, ReadError,
};

use crate::archive::{ArchiveReader, LaneArchives, LevelError, NOT_FOUND, Stamp, find_entry};

const IMAGE_PREVIEW_LIMIT: u64 = 64 * 1024 * 1024;

// The archive previewed last, kept open so that moving through its entries
// does not build its index again: for a compressed tar that means
// decompressing all of it. A changed archive misses it, and previewing a
// file outside any archive or `Effect::ReleasePreview` lets it go so the
// archive is not held open.
#[derive(Default)]
pub(crate) struct PreviewedArchive(Option<OpenArchive>);

struct OpenArchive {
    path: PathBuf,
    stamp: Stamp,
    outer: Vec<OuterArchive>,
    reader: Box<dyn ArchiveReader>,
}

// Decoding is the core's pure function, but it runs here on the read thread
// so that decoding a file of the full preview size does not hold up the UI.
pub(crate) fn load_preview(
    source: &PreviewSource,
    lane: LaneArchives<'_>,
    previewed: &mut PreviewedArchive,
    cancelled: &dyn Fn() -> bool,
) -> Result<Document, ReadError> {
    let (bytes, truncated) = read_source(source, PREVIEW_LIMIT, lane, previewed, cancelled)?;
    Ok(Document::decode(bytes, truncated, source.encoding))
}

pub(crate) fn load_image(
    source: &PreviewSource,
    lane: LaneArchives<'_>,
    previewed: &mut PreviewedArchive,
    cancelled: &dyn Fn() -> bool,
) -> Result<ImageData, ReadError> {
    load_image_limited(source, IMAGE_PREVIEW_LIMIT, lane, previewed, cancelled)
}

fn decode_image(bytes: &[u8]) -> ImageResult<ImageData> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let image = image.into_rgba8();
    let (width, height) = image.dimensions();
    Ok(ImageData {
        width,
        height,
        rgba: image.into_raw(),
    })
}

fn load_image_limited(
    source: &PreviewSource,
    limit: u64,
    lane: LaneArchives<'_>,
    previewed: &mut PreviewedArchive,
    cancelled: &dyn Fn() -> bool,
) -> Result<ImageData, ReadError> {
    let (bytes, truncated) = read_source(source, limit, lane, previewed, cancelled)?;
    if truncated {
        return Err(ReadError::Failed(IoError::new(format!(
            "too large (over {} MiB)",
            IMAGE_PREVIEW_LIMIT / 1024 / 1024
        ))));
    }
    decode_image(&bytes).map_err(|error| ReadError::Failed(IoError::new(error.to_string())))
}

// At most `limit` bytes, and whether there were more.
fn read_source(
    source: &PreviewSource,
    limit: u64,
    lane: LaneArchives<'_>,
    previewed: &mut PreviewedArchive,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Vec<u8>, bool), ReadError> {
    let Some((archive, name)) = &source.entry else {
        previewed.0 = None;
        lane.release();
        return read_limited(File::open(&source.path).map_err(|e| failed(&e))?, limit)
            .map_err(|e| failed(&e));
    };
    let stamp = Stamp::of(&source.path).map_err(|e| failed(&e))?;
    lane.hold(&source.path, stamp, &archive.outer);
    let open = match &mut previewed.0 {
        Some(open)
            if open.path == source.path && open.stamp == stamp && open.outer == archive.outer =>
        {
            open
        }
        slot => {
            *slot = None;
            let opened = lane
                .open(&source.path, stamp, archive, &source.passwords, cancelled)
                .map_err(LevelError::into_read_error)?;
            slot.insert(OpenArchive {
                path: source.path.clone(),
                stamp,
                outer: archive.outer.clone(),
                reader: opened.reader,
            })
        }
    };
    let index = find_entry(open.reader.entries(), &archive.inner, name)
        .ok_or_else(|| ReadError::Failed(IoError::new(NOT_FOUND)))?;
    let level = archive.outer.len();
    let result = open
        .reader
        .open(index, source.passwords.get(level))
        .map_err(|error| LevelError { level, error }.into_read_error())
        .and_then(|entry| read_limited(entry, limit).map_err(|e| failed(&e)));
    // A read that failed partway may leave a tar's stream out of step.
    if result.is_err() {
        previewed.0 = None;
    }
    result
}

fn read_limited(reader: impl Read, limit: u64) -> io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let truncated = bytes.len() > limit;
    bytes.truncate(limit);
    Ok((bytes, truncated))
}

fn failed(error: &io::Error) -> ReadError {
    ReadError::Failed(IoError::new(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    use image::{ImageFormat, RgbaImage};
    use tempfile::TempDir;
    use wafu_core::{ArchiveFormat, ArchivePath, Passwords};

    use super::*;
    use crate::archive::tests::{archive_bytes, encrypted_bytes, write_archive, write_encrypted};
    use crate::archive::{NestedArchives, create_archive};

    #[test]
    fn a_file_is_read_up_to_the_limit() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"0123456789").unwrap();
        let source = file_source(&path);

        assert_eq!(
            read(&source, 100, &mut PreviewedArchive::default()).unwrap(),
            (b"0123456789".to_vec(), false)
        );
        assert_eq!(
            read(&source, 10, &mut PreviewedArchive::default()).unwrap(),
            (b"0123456789".to_vec(), false)
        );
        assert_eq!(
            read(&source, 4, &mut PreviewedArchive::default()).unwrap(),
            (b"0123".to_vec(), true)
        );
    }

    #[test]
    fn a_text_file_becomes_a_document() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"one\ntwo\n").unwrap();

        let document = preview(&file_source(&path), &mut PreviewedArchive::default()).unwrap();

        assert!(document.is_text);
        assert_eq!(texts(&document), ["one", "two"]);
        assert!(!document.truncated);
    }

    #[test]
    fn an_archive_entry_is_read_by_name_inside_its_directory() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        write_zip(&archive, None);

        let document = preview(
            &entry_source(&archive, &["d"], "f.txt", None),
            &mut PreviewedArchive::default(),
        )
        .unwrap();
        assert_eq!(texts(&document), ["inner"]);

        let error = preview(
            &entry_source(&archive, &[], "f.txt", None),
            &mut PreviewedArchive::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ReadError::Failed(IoError::new("not found in the archive"))
        );
    }

    #[test]
    fn an_encrypted_entry_asks_for_a_password() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("p.zip");
        write_zip(&archive, Some("secret"));
        let mut previewed = PreviewedArchive::default();

        assert_eq!(
            preview(
                &entry_source(&archive, &[], "top.txt", None),
                &mut previewed
            )
            .unwrap_err(),
            ReadError::PasswordRequired {
                level: 0,
                retry: false
            }
        );
        assert_eq!(
            preview(
                &entry_source(&archive, &[], "top.txt", Some("wrong")),
                &mut previewed
            )
            .unwrap_err(),
            ReadError::PasswordRequired {
                level: 0,
                retry: true
            }
        );
        let document = preview(
            &entry_source(&archive, &[], "top.txt", Some("secret")),
            &mut previewed,
        )
        .unwrap();
        assert_eq!(texts(&document), ["top"]);
    }

    #[test]
    fn an_archive_is_kept_open_between_previews_until_it_changes() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.tar");
        write_tar(&archive, b"first");
        let mut previewed = PreviewedArchive::default();
        let source = entry_source(&archive, &[], "a.txt", None);
        assert_eq!(texts(&preview(&source, &mut previewed).unwrap()), ["first"]);

        // The first header, which a new index would have to read, made
        // unreadable without the archive looking changed.
        let modified = fs::metadata(&archive).unwrap().modified().unwrap();
        let file = File::options().write(true).open(&archive).unwrap();
        (&file).write_all(&[0; 512]).unwrap();
        file.set_modified(modified).unwrap();
        drop(file);
        let other = entry_source(&archive, &[], "b.txt", None);
        assert_eq!(texts(&preview(&other, &mut previewed).unwrap()), ["b"]);

        // Longer by a block, so the archive changes even within the
        // resolution of its time.
        let second = "second".repeat(100);
        write_tar(&archive, second.as_bytes());
        assert_eq!(texts(&preview(&source, &mut previewed).unwrap()), [second]);
    }

    #[test]
    fn an_entry_of_a_nested_archive_is_read_through_its_outer_archive() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let nested = archive_bytes(ArchiveFormat::TarGz, &[("d/f.txt", b"nested")]);
        write_archive(&archive, ArchiveFormat::Zip, &[("x/n.tar.gz", &nested)]);
        let mut source = entry_source(&archive, &["d"], "f.txt", None);
        if let Some((path, _)) = &mut source.entry {
            path.outer = vec![OuterArchive {
                format: Some(ArchiveFormat::Zip),
                entry: vec!["x".to_owned(), "n.tar.gz".to_owned()],
            }];
            path.format = Some(ArchiveFormat::TarGz);
        }
        let mut previewed = PreviewedArchive::default();

        let document = preview(&source, &mut previewed).unwrap();
        assert_eq!(texts(&document), ["nested"]);

        let outer = entry_source(&archive, &["x"], "n.tar.gz", None);
        let (bytes, _) = read(&outer, PREVIEW_LIMIT, &mut previewed).unwrap();
        assert_eq!(bytes, nested);
    }

    #[test]
    fn an_encrypted_entry_of_an_encrypted_nested_archive_takes_the_password_of_each_level() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let nested = encrypted_bytes(ArchiveFormat::Zip, Some("inner"), &[("f.txt", b"f")]);
        write_encrypted(
            &archive,
            ArchiveFormat::Zip,
            Some("outer"),
            &[("n.zip", &nested)],
        );
        let mut source = entry_source(&archive, &[], "f.txt", Some("outer"));
        if let Some((path, _)) = &mut source.entry {
            path.outer = vec![OuterArchive {
                format: Some(ArchiveFormat::Zip),
                entry: vec!["n.zip".to_owned()],
            }];
        }
        let mut previewed = PreviewedArchive::default();

        assert_eq!(
            preview(&source, &mut previewed).unwrap_err(),
            ReadError::PasswordRequired {
                level: 1,
                retry: false
            }
        );

        source.passwords.set(1, Some("inner".to_owned()));
        let document = preview(&source, &mut previewed).unwrap();
        assert_eq!(texts(&document), ["f"]);
    }

    #[test]
    fn an_image_over_the_limit_is_too_large() {
        let mut bytes = Cursor::new(Vec::new());
        RgbaImage::new(2, 3)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let bytes = bytes.into_inner();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.png");
        fs::write(&path, &bytes).unwrap();
        let source = file_source(&path);
        let len = bytes.len() as u64;

        let image = image_limited(&source, len, &mut PreviewedArchive::default()).unwrap();
        assert_eq!((image.width, image.height), (2, 3));

        assert_eq!(
            image_limited(&source, len - 1, &mut PreviewedArchive::default()).unwrap_err(),
            ReadError::Failed(IoError::new("too large (over 64 MiB)"))
        );
    }

    fn preview(
        source: &PreviewSource,
        previewed: &mut PreviewedArchive,
    ) -> Result<Document, ReadError> {
        let nested = NestedArchives::default();
        load_preview(
            source,
            nested.lane("preview", &AtomicBool::new(false)),
            previewed,
            &|| false,
        )
    }

    fn image_limited(
        source: &PreviewSource,
        limit: u64,
        previewed: &mut PreviewedArchive,
    ) -> Result<ImageData, ReadError> {
        let nested = NestedArchives::default();
        load_image_limited(
            source,
            limit,
            nested.lane("preview", &AtomicBool::new(false)),
            previewed,
            &|| false,
        )
    }

    fn read(
        source: &PreviewSource,
        limit: u64,
        previewed: &mut PreviewedArchive,
    ) -> Result<(Vec<u8>, bool), ReadError> {
        let nested = NestedArchives::default();
        read_source(
            source,
            limit,
            nested.lane("preview", &AtomicBool::new(false)),
            previewed,
            &|| false,
        )
    }

    fn texts(document: &Document) -> Vec<String> {
        document
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect()
    }

    fn file_source(path: &Path) -> PreviewSource {
        PreviewSource {
            path: path.to_path_buf(),
            entry: None,
            passwords: Passwords::default(),
            encoding: None,
        }
    }

    fn entry_source(
        archive: &Path,
        inner: &[&str],
        name: &str,
        password: Option<&str>,
    ) -> PreviewSource {
        PreviewSource {
            path: archive.to_path_buf(),
            entry: Some((
                ArchivePath {
                    outer: Vec::new(),
                    format: ArchiveFormat::from_name(archive.file_name().unwrap()),
                    inner: inner.iter().map(|part| (*part).to_owned()).collect(),
                },
                name.to_owned(),
            )),
            passwords: {
                let mut passwords = Passwords::default();
                passwords.set(0, password.map(ToOwned::to_owned));
                passwords
            },
            encoding: None,
        }
    }

    // `a.txt` holding `a` and then `b.txt`.
    fn write_tar(path: &Path, a: &[u8]) {
        let mut writer =
            create_archive(File::create(path).unwrap(), ArchiveFormat::Tar, None).unwrap();
        writer
            .add_file("a.txt", a.len() as u64, None, None, &mut &a[..])
            .unwrap();
        writer
            .add_file("b.txt", 1, None, None, &mut &b"b"[..])
            .unwrap();
        writer.finish().unwrap();
    }

    // `top.txt` at the root and `d/f.txt` below it.
    fn write_zip(path: &Path, password: Option<&str>) {
        let mut writer =
            create_archive(File::create(path).unwrap(), ArchiveFormat::Zip, password).unwrap();
        writer
            .add_file("top.txt", 3, None, None, &mut &b"top"[..])
            .unwrap();
        writer
            .add_file("d/f.txt", 5, None, None, &mut &b"inner"[..])
            .unwrap();
        writer.finish().unwrap();
    }
}
