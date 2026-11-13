use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

use wafu_core::{ConfigContent, ConfigFile, ConfigSource, IoError, read_config_chain};

/// Reads the configuration file and the files it extends, writing the
/// template first when the configuration file itself is missing. `home` is
/// what `~` in `extends` stands for.
#[must_use]
pub fn read_or_create(path: &Path, template: &str, home: &Path) -> ConfigSource {
    let first = match fs::read_to_string(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return match write_template(path, template) {
                Ok(()) => ConfigSource::Created,
                Err(error) => ConfigSource::Read(vec![ConfigFile {
                    path: path.to_path_buf(),
                    content: ConfigContent::Unreadable(IoError::new(error.to_string())),
                }]),
            };
        }
        result => result,
    };
    let mut first = Some(first.map_err(|error| IoError::new(error.to_string())));
    ConfigSource::Read(read_config_chain(path, home, |file| {
        first.take().unwrap_or_else(|| read_text(file))
    }))
}

pub(crate) fn read_text(path: &Path) -> Result<String, IoError> {
    fs::read_to_string(path).map_err(|error| IoError::new(error.to_string()))
}

fn write_template(path: &Path, template: &str) -> io::Result<()> {
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

        assert_eq!(
            read_or_create(&path, "# template\n", dir.path()),
            ConfigSource::Created
        );

        assert_eq!(fs::read_to_string(&path).unwrap(), "# template\n");
        assert_eq!(
            read_or_create(&path, "# other\n", dir.path()),
            ConfigSource::Read(vec![text(&path, "# template\n")])
        );
    }

    #[test]
    fn an_unreadable_file_is_reported_and_left_alone() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, b"\xff").unwrap();

        let source = read_or_create(&path, "# template\n", dir.path());

        assert!(matches!(
            source.clone(),
            ConfigSource::Read(files) if matches!(
                files.as_slice(),
                [ConfigFile { content: ConfigContent::Unreadable(_), .. }]
            )
        ));
        assert_eq!(fs::read(&path).unwrap(), b"\xff");
    }

    #[test]
    fn the_files_a_config_extends_are_read_with_it() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        fs::create_dir(&home).unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "extends = \"~/common.toml\"\n").unwrap();
        let common = home.join("common.toml");
        fs::write(&common, "x = 1\n").unwrap();

        assert_eq!(
            read_or_create(&path, "# template\n", &home),
            ConfigSource::Read(vec![
                text(&common, "x = 1\n"),
                text(&path, "extends = \"~/common.toml\"\n"),
            ])
        );
    }

    fn text(path: &Path, text: &str) -> ConfigFile {
        ConfigFile {
            path: path.to_path_buf(),
            content: ConfigContent::Text(text.to_owned()),
        }
    }
}
