use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use wafu_core::{ConfigContent, ConfigFile, ConfigSource, IoError, config_extends};

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
    let mut first = Some(first);
    ConfigSource::Read(read_chain(path, home, &mut |file| {
        first.take().unwrap_or_else(|| fs::read_to_string(file))
    }))
}

// The files in the order they merge, as for `IoEvent::ConfigLoaded`. A file
// that cannot be read, or is reached again while its own bases are being
// read, is listed without following it further.
pub(crate) fn read_chain(
    path: &Path,
    home: &Path,
    read: &mut dyn FnMut(&Path) -> io::Result<String>,
) -> Vec<ConfigFile> {
    let mut files = Vec::new();
    visit(path, home, read, &mut Vec::new(), &mut files);
    files
}

fn visit(
    path: &Path,
    home: &Path,
    read: &mut dyn FnMut(&Path) -> io::Result<String>,
    open: &mut Vec<PathBuf>,
    files: &mut Vec<ConfigFile>,
) {
    let path = path.to_path_buf();
    if open.contains(&path) {
        files.push(ConfigFile {
            path,
            content: ConfigContent::Cycle,
        });
        return;
    }
    let text = match read(&path) {
        Ok(text) => text,
        Err(error) => {
            files.push(ConfigFile {
                path,
                content: ConfigContent::Unreadable(IoError::new(error.to_string())),
            });
            return;
        }
    };
    open.push(path.clone());
    for base in config_extends(&text, &path, home) {
        visit(&base, home, read, open, files);
    }
    open.pop();
    files.push(ConfigFile {
        path,
        content: ConfigContent::Text(text),
    });
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
    fn bases_come_before_the_file_that_extends_them_in_the_order_written() {
        let dir = TempDir::new().unwrap();
        let main = write(
            dir.path(),
            "config.toml",
            "extends = [\"a.toml\", \"b.toml\"]\n",
        );
        let a = write(dir.path(), "a.toml", "extends = \"c.toml\"\n");
        let b = write(dir.path(), "b.toml", "x = 1\n");
        let c = write(dir.path(), "c.toml", "y = 1\n");

        assert_eq!(
            chain(&main, dir.path()),
            [
                text(&c, "y = 1\n"),
                text(&a, "extends = \"c.toml\"\n"),
                text(&b, "x = 1\n"),
                text(&main, "extends = [\"a.toml\", \"b.toml\"]\n"),
            ]
        );
    }

    #[test]
    fn a_relative_base_is_taken_from_the_directory_of_the_file_naming_it() {
        let dir = TempDir::new().unwrap();
        let main = write(dir.path(), "config.toml", "extends = \"shared/a.toml\"\n");
        fs::create_dir(dir.path().join("shared")).unwrap();
        let a = write(
            &dir.path().join("shared"),
            "a.toml",
            "extends = \"b.toml\"\n",
        );
        let b = write(&dir.path().join("shared"), "b.toml", "");

        let paths: Vec<PathBuf> = chain(&main, dir.path())
            .into_iter()
            .map(|file| file.path)
            .collect();

        assert_eq!(paths, [b, a, main]);
    }

    #[test]
    fn a_tilde_in_extends_stands_for_the_home_given() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        fs::create_dir(&home).unwrap();
        let main = write(dir.path(), "config.toml", "extends = \"~/common.toml\"\n");
        let common = write(&home, "common.toml", "x = 1\n");

        assert_eq!(chain(&main, &home)[0], text(&common, "x = 1\n"));
    }

    #[test]
    fn a_missing_base_is_listed_as_unreadable_and_the_rest_still_read() {
        let dir = TempDir::new().unwrap();
        let main = write(
            dir.path(),
            "config.toml",
            "extends = [\"missing.toml\", \"b.toml\"]\n",
        );
        let b = write(dir.path(), "b.toml", "");

        let files = chain(&main, dir.path());

        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, dir.path().join("missing.toml"));
        assert!(matches!(files[0].content, ConfigContent::Unreadable(_)));
        assert_eq!(files[1], text(&b, ""));
        assert_eq!(files[2].path, main);
    }

    #[test]
    fn a_file_reached_again_while_it_is_being_read_is_a_cycle() {
        let dir = TempDir::new().unwrap();
        let main = write(dir.path(), "config.toml", "extends = \"a.toml\"\n");
        let a = write(dir.path(), "a.toml", "extends = \"./config.toml\"\n");

        assert_eq!(
            chain(&main, dir.path()),
            [
                ConfigFile {
                    path: main.clone(),
                    content: ConfigContent::Cycle,
                },
                text(&a, "extends = \"./config.toml\"\n"),
                text(&main, "extends = \"a.toml\"\n"),
            ]
        );
    }

    #[test]
    fn a_file_reached_through_two_branches_is_read_for_each() {
        let dir = TempDir::new().unwrap();
        let main = write(
            dir.path(),
            "config.toml",
            "extends = [\"a.toml\", \"b.toml\"]\n",
        );
        let a = write(dir.path(), "a.toml", "extends = \"c.toml\"\n");
        let b = write(dir.path(), "b.toml", "extends = \"c.toml\"\n");
        let c = write(dir.path(), "c.toml", "");

        let paths: Vec<PathBuf> = chain(&main, dir.path())
            .into_iter()
            .map(|file| file.path)
            .collect();

        assert_eq!(paths, [c.clone(), a, c, b, main]);
    }

    fn chain(path: &Path, home: &Path) -> Vec<ConfigFile> {
        read_chain(path, home, &mut |file| fs::read_to_string(file))
    }

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        path
    }

    fn text(path: &Path, text: &str) -> ConfigFile {
        ConfigFile {
            path: path.to_path_buf(),
            content: ConfigContent::Text(text.to_owned()),
        }
    }
}
