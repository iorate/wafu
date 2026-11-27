// An integration test, so that the package can leave it out: the published crate does not contain docs/.

use std::io::ErrorKind;
use std::path::Path;
use std::{env, fs};

const REGENERATE: &str = "WAFU_UPDATE_DOCS=1 cargo test -p wafu-core --test commands_doc";

#[test]
fn commands_doc_is_up_to_date() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/commands.md");
    let expected = wafu_core::commands_doc();
    let actual = match fs::read_to_string(&path) {
        Ok(text) => text.replace("\r\n", "\n"),
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => panic!("cannot read {}: {error}", path.display()),
    };
    if actual == expected {
        return;
    }
    if env::var("WAFU_UPDATE_DOCS").as_deref() == Ok("1") {
        fs::write(&path, expected).unwrap();
        return;
    }
    panic!("docs/commands.md is missing or out of date; regenerate it with: {REGENERATE}");
}
