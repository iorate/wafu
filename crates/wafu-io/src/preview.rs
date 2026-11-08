use std::fs::{self, File};
use std::io::{self, Cursor, Read};
use std::path::PathBuf;
use std::time::SystemTime;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, ImageResult};
use wafu_core::{Document, ImageData, IoError, PREVIEW_LIMIT, PreviewError, PreviewSource};

use crate::archive::{ArchiveError, ArchiveReader, find_entry, open_archive};

const IMAGE_PREVIEW_LIMIT: u64 = 64 * 1024 * 1024;

// The archive previewed last, kept open so that moving through its entries
// does not build its index again: for a compressed tar that means
// decompressing all of it. A changed archive misses it, and previewing a
// file outside any archive lets it go so the archive is not held open.
#[derive(Default)]
pub(crate) struct PreviewedArchive(Option<OpenArchive>);

struct OpenArchive {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    reader: Box<dyn ArchiveReader>,
}

// Decoding is the core's pure function, but it runs here on the read thread
// so that decoding a file of the full preview size does not hold up the UI.
pub(crate) fn load_preview(
    source: &PreviewSource,
    previewed: &mut PreviewedArchive,
) -> Result<Document, PreviewError> {
    let (bytes, truncated) = read_source(source, PREVIEW_LIMIT, previewed)?;
    Ok(Document::decode(bytes, truncated))
}

pub(crate) fn load_image(
    source: &PreviewSource,
    previewed: &mut PreviewedArchive,
) -> Result<ImageData, PreviewError> {
    load_image_limited(source, IMAGE_PREVIEW_LIMIT, previewed)
}

fn decode_image(bytes: &[u8]) -> ImageResult<ImageData> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let format = reader.format().map_or("", format_name);
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
        format,
    })
}

fn format_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::Gif => "gif",
        ImageFormat::Bmp => "bmp",
        ImageFormat::WebP => "webp",
        ImageFormat::Ico => "ico",
        other => other.extensions_str().first().copied().unwrap_or(""),
    }
}

fn load_image_limited(
    source: &PreviewSource,
    limit: u64,
    previewed: &mut PreviewedArchive,
) -> Result<ImageData, PreviewError> {
    let (bytes, truncated) = read_source(source, limit, previewed)?;
    if truncated {
        return Err(PreviewError::Failed(IoError::new(format!(
            "too large (over {} MiB)",
            IMAGE_PREVIEW_LIMIT / 1024 / 1024
        ))));
    }
    decode_image(&bytes).map_err(|error| PreviewError::Failed(IoError::new(error.to_string())))
}

// At most `limit` bytes, and whether there were more.
fn read_source(
    source: &PreviewSource,
    limit: u64,
    previewed: &mut PreviewedArchive,
) -> Result<(Vec<u8>, bool), PreviewError> {
    let Some((archive, name)) = &source.entry else {
        previewed.0 = None;
        return read_limited(File::open(&source.path).map_err(|e| failed(&e))?, limit)
            .map_err(|e| failed(&e));
    };
    let meta = fs::metadata(&source.path).map_err(|e| failed(&e))?;
    let (len, modified) = (meta.len(), meta.modified().ok());
    let open = match &mut previewed.0 {
        Some(open) if open.path == source.path && open.len == len && open.modified == modified => {
            open
        }
        slot => {
            *slot = None;
            slot.insert(OpenArchive {
                path: source.path.clone(),
                len,
                modified,
                reader: open_archive(&source.path, archive.format).map_err(convert)?,
            })
        }
    };
    let index = find_entry(open.reader.entries(), &archive.inner, name)
        .ok_or_else(|| PreviewError::Failed(IoError::new("not found in the archive")))?;
    let result = open
        .reader
        .open(index, source.password.as_deref())
        .map_err(convert)
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

fn convert(error: ArchiveError) -> PreviewError {
    match error {
        ArchiveError::PasswordRequired => PreviewError::PasswordRequired { retry: false },
        ArchiveError::WrongPassword => PreviewError::PasswordRequired { retry: true },
        other => PreviewError::Failed(IoError::new(other.to_string())),
    }
}

fn failed(error: &io::Error) -> PreviewError {
    PreviewError::Failed(IoError::new(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::Path;

    use image::{ImageFormat, RgbaImage};
    use tempfile::TempDir;
    use wafu_core::{ArchiveFormat, ArchivePath};

    use super::*;
    use crate::archive::create_archive;

    #[test]
    fn a_file_is_read_up_to_the_limit() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"0123456789").unwrap();
        let source = file_source(&path);

        assert_eq!(
            read_source(&source, 100, &mut PreviewedArchive::default()).unwrap(),
            (b"0123456789".to_vec(), false)
        );
        assert_eq!(
            read_source(&source, 10, &mut PreviewedArchive::default()).unwrap(),
            (b"0123456789".to_vec(), false)
        );
        assert_eq!(
            read_source(&source, 4, &mut PreviewedArchive::default()).unwrap(),
            (b"0123".to_vec(), true)
        );
    }

    #[test]
    fn a_text_file_becomes_a_document() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"one\ntwo\n").unwrap();

        let document = load_preview(&file_source(&path), &mut PreviewedArchive::default()).unwrap();

        assert!(document.is_text);
        assert_eq!(texts(&document), ["one", "two"]);
        assert!(!document.truncated);
    }

    #[test]
    fn an_archive_entry_is_read_by_name_inside_its_directory() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        write_zip(&archive, None);

        let document = load_preview(
            &entry_source(&archive, &["d"], "f.txt", None),
            &mut PreviewedArchive::default(),
        )
        .unwrap();
        assert_eq!(texts(&document), ["inner"]);

        let error = load_preview(
            &entry_source(&archive, &[], "f.txt", None),
            &mut PreviewedArchive::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            PreviewError::Failed(IoError::new("not found in the archive"))
        );
    }

    #[test]
    fn an_encrypted_entry_asks_for_a_password() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("p.zip");
        write_zip(&archive, Some("secret"));
        let mut previewed = PreviewedArchive::default();

        assert_eq!(
            load_preview(
                &entry_source(&archive, &[], "top.txt", None),
                &mut previewed
            )
            .unwrap_err(),
            PreviewError::PasswordRequired { retry: false }
        );
        assert_eq!(
            load_preview(
                &entry_source(&archive, &[], "top.txt", Some("wrong")),
                &mut previewed
            )
            .unwrap_err(),
            PreviewError::PasswordRequired { retry: true }
        );
        let document = load_preview(
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
        assert_eq!(
            texts(&load_preview(&source, &mut previewed).unwrap()),
            ["first"]
        );

        // The first header, which a new index would have to read, made
        // unreadable without the archive looking changed.
        let modified = fs::metadata(&archive).unwrap().modified().unwrap();
        let file = File::options().write(true).open(&archive).unwrap();
        (&file).write_all(&[0; 512]).unwrap();
        file.set_modified(modified).unwrap();
        drop(file);
        let other = entry_source(&archive, &[], "b.txt", None);
        assert_eq!(texts(&load_preview(&other, &mut previewed).unwrap()), ["b"]);

        // Longer by a block, so the archive changes even within the
        // resolution of its time.
        let second = "second".repeat(100);
        write_tar(&archive, second.as_bytes());
        assert_eq!(
            texts(&load_preview(&source, &mut previewed).unwrap()),
            [second]
        );
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

        let image = load_image_limited(&source, len, &mut PreviewedArchive::default()).unwrap();
        assert_eq!((image.width, image.height), (2, 3));

        assert_eq!(
            load_image_limited(&source, len - 1, &mut PreviewedArchive::default()).unwrap_err(),
            PreviewError::Failed(IoError::new("too large (over 64 MiB)"))
        );
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
            password: None,
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
                    format: ArchiveFormat::from_name(archive.file_name().unwrap()),
                    inner: inner.iter().map(|part| (*part).to_owned()).collect(),
                },
                name.to_owned(),
            )),
            password: password.map(ToOwned::to_owned),
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
