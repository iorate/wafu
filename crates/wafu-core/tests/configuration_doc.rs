// An integration test, so that the package can leave it out: the published crate does not contain docs/.

use std::path::Path;
use std::{env, fs};

const REGENERATE: &str = "WAFU_UPDATE_DOCS=1 cargo test -p wafu-core --test configuration_doc";

#[test]
fn configuration_doc_is_up_to_date() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/configuration.md");
    let actual = match fs::read_to_string(&path) {
        Ok(text) => text.replace("\r\n", "\n"),
        Err(error) => panic!("cannot read {}: {error}", path.display()),
    };
    let expected = match wafu_core::configuration_doc(&actual) {
        Ok(text) => text,
        Err(error) => panic!("docs/configuration.md: {error}"),
    };
    if actual == expected {
        return;
    }
    if env::var("WAFU_UPDATE_DOCS").as_deref() == Ok("1") {
        fs::write(&path, expected).unwrap();
        return;
    }
    panic!("docs/configuration.md is out of date; regenerate it with: {REGENERATE}");
}
