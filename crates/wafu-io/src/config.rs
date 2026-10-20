use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use wafu_core::{ConfigSource, IoError};

/// Reads the configuration file, writing the template first when there is none.
#[must_use]
pub fn read_or_create(path: &Path, template: &str) -> ConfigSource {
    match fs::read_to_string(path) {
        Ok(text) => ConfigSource::Text(text),
        Err(error) if error.kind() == ErrorKind::NotFound => match write_template(path, template) {
            Ok(()) => ConfigSource::Created,
            Err(error) => ConfigSource::Unreadable(IoError::new(error.to_string())),
        },
        Err(error) => ConfigSource::Unreadable(IoError::new(error.to_string())),
    }
}

fn write_template(path: &Path, template: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, template)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_missing_file_is_created_from_the_template() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sub").join("config.toml");

        assert_eq!(read_or_create(&path, "# template\n"), ConfigSource::Created);

        assert_eq!(fs::read_to_string(&path).unwrap(), "# template\n");
        assert_eq!(
            read_or_create(&path, "# other\n"),
            ConfigSource::Text("# template\n".to_owned())
        );
    }

    #[test]
    fn an_unreadable_file_is_reported() {
        let dir = TempDir::new().unwrap();

        // A directory cannot be read as a file.
        let source = read_or_create(dir.path(), "");

        assert!(matches!(source, ConfigSource::Unreadable(_)));
    }
}
