use std::fs::File;
use std::io::{self, Cursor, Read};

use image::{DynamicImage, ImageDecoder, ImageReader, ImageResult};
use wafu_core::{Document, ImageData, IoError, PREVIEW_LIMIT, PreviewError, PreviewSource};

use crate::archive::{ArchiveError, find_entry, open_archive};

/// Reads the start of the file or archive entry and decodes it for the preview.
pub fn load_preview(source: &PreviewSource) -> Result<Document, PreviewError> {
    let (bytes, truncated) = read_source(source, PREVIEW_LIMIT)?;
    Ok(Document::decode(bytes, truncated))
}

/// Reads and decodes the whole image, with its orientation applied.
pub fn load_image(source: &PreviewSource) -> Result<ImageData, PreviewError> {
    let (bytes, _) = read_source(source, u64::MAX)?;
    decode_image(&bytes).map_err(|error| PreviewError::Failed(IoError::new(error.to_string())))
}

/// Decodes an image from its file contents.
pub fn decode_image(bytes: &[u8]) -> ImageResult<ImageData> {
    let mut decoder = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .into_decoder()?;
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

// At most `limit` bytes, and whether there were more.
fn read_source(source: &PreviewSource, limit: u64) -> Result<(Vec<u8>, bool), PreviewError> {
    match &source.entry {
        None => read_limited(File::open(&source.path).map_err(|e| failed(&e))?, limit)
            .map_err(|e| failed(&e)),
        Some((archive, name)) => {
            let mut reader = open_archive(&source.path, archive.format).map_err(convert)?;
            let index = find_entry(reader.entries(), &archive.inner, name)
                .ok_or_else(|| PreviewError::Failed(IoError::new("not found in the archive")))?;
            let entry = reader
                .open(index, source.password.as_deref())
                .map_err(convert)?;
            read_limited(entry, limit).map_err(|e| failed(&e))
        }
    }
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
    use std::fs;
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
            read_source(&source, 100).unwrap(),
            (b"0123456789".to_vec(), false)
        );
        assert_eq!(
            read_source(&source, 10).unwrap(),
            (b"0123456789".to_vec(), false)
        );
        assert_eq!(read_source(&source, 4).unwrap(), (b"0123".to_vec(), true));
    }

    #[test]
    fn a_missing_file_fails() {
        let dir = TempDir::new().unwrap();

        let error = load_preview(&file_source(&dir.path().join("none"))).unwrap_err();

        assert!(matches!(error, PreviewError::Failed(_)));
    }

    #[test]
    fn a_text_file_becomes_a_document() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"one\ntwo\n").unwrap();

        let document = load_preview(&file_source(&path)).unwrap();

        assert!(document.is_text);
        assert_eq!(texts(&document), ["one", "two"]);
        assert!(!document.truncated);
    }

    #[test]
    fn an_archive_entry_is_read_by_name_inside_its_directory() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        write_zip(&archive, None);

        let document = load_preview(&entry_source(&archive, &["d"], "f.txt", None)).unwrap();
        assert_eq!(texts(&document), ["inner"]);

        let error = load_preview(&entry_source(&archive, &[], "f.txt", None)).unwrap_err();
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

        assert_eq!(
            load_preview(&entry_source(&archive, &[], "top.txt", None)).unwrap_err(),
            PreviewError::PasswordRequired { retry: false }
        );
        assert_eq!(
            load_preview(&entry_source(&archive, &[], "top.txt", Some("wrong"))).unwrap_err(),
            PreviewError::PasswordRequired { retry: true }
        );
        let document =
            load_preview(&entry_source(&archive, &[], "top.txt", Some("secret"))).unwrap();
        assert_eq!(texts(&document), ["top"]);
    }

    #[test]
    fn a_png_decodes_to_rgba() {
        let mut image = RgbaImage::new(2, 3);
        image.put_pixel(1, 2, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();

        let decoded = decode_image(bytes.get_ref()).unwrap();

        assert_eq!((decoded.width, decoded.height), (2, 3));
        assert_eq!(decoded.rgba.len(), 2 * 3 * 4);
        assert_eq!(&decoded.rgba[(2 * 2 + 1) * 4..], [10, 20, 30, 255]);
    }

    #[test]
    fn bytes_that_are_not_an_image_fail() {
        assert!(decode_image(b"not an image").is_err());

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.png");
        fs::write(&path, b"not an image").unwrap();
        assert!(matches!(
            load_image(&file_source(&path)),
            Err(PreviewError::Failed(_))
        ));
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
                    format: ArchiveFormat::Zip,
                    inner: inner.iter().map(|part| (*part).to_owned()).collect(),
                },
                name.to_owned(),
            )),
            password: password.map(ToOwned::to_owned),
        }
    }

    // `top.txt` at the root and `d/f.txt` below it.
    fn write_zip(path: &Path, password: Option<&str>) {
        let mut writer = create_archive(File::create(path).unwrap(), ArchiveFormat::Zip, password);
        writer.start_file("top.txt", 3, None, None).unwrap();
        writer.write_all(b"top").unwrap();
        writer.start_file("d/f.txt", 5, None, None).unwrap();
        writer.write_all(b"inner").unwrap();
        writer.finish().unwrap();
    }
}
