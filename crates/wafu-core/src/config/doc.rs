use std::fmt::Write;

use super::Config;
use super::settings::{
    COLORS, COLORS_CONFIG, FILELIST, FONT, IMAGE_PREVIEW, LOG, PROGRAMS, Setting, TEXT_PREVIEW,
};
use crate::app::Os;

const BEGIN: (&str, &str) = ("<!-- generated:", " -->");
const END: &str = "<!-- /generated -->";

const REGIONS: &[Region] = &[
    ("filelist", || {
        item_table(FILELIST, |config| &config.filelist)
    }),
    ("preview.text", || {
        item_table(TEXT_PREVIEW, |config| &config.preview.text)
    }),
    ("preview.image", || {
        item_table(IMAGE_PREVIEW, |config| &config.preview.image)
    }),
    ("log", || item_table(LOG, |config| &config.log)),
    ("programs", || {
        item_table(PROGRAMS, |config| &config.programs)
    }),
    // The fonts of the areas have no defaults of their own.
    ("font", || item_table(FONT, |config| &config.font.default)),
    ("colors", || {
        item_table(COLORS_CONFIG, |config| &config.colors)
    }),
    ("colors.themes", color_table),
];

const OSES: [(Os, &str); 3] = [
    (Os::Windows, "Windows"),
    (Os::MacOs, "macOS"),
    (Os::Linux, "Linux"),
];

type Region = (&'static str, fn() -> String);

/// `current`, the text of `docs/configuration.md`, with what stands between
/// each pair of `generated` comments written anew from the settings: the
/// tables of items with their defaults. The same on every OS.
///
/// # Errors
///
/// Returns what is wrong with the comments: one names no region or a region
/// already written, one is not closed or closes nothing, or a region has
/// none.
pub fn configuration_doc(current: &str) -> Result<String, String> {
    let mut out = String::with_capacity(current.len());
    let mut written = Vec::new();
    let mut open = None;
    for line in current.split_inclusive('\n') {
        let text = line.strip_suffix('\n').unwrap_or(line);
        let begin = text
            .strip_prefix(BEGIN.0)
            .and_then(|rest| rest.strip_suffix(BEGIN.1));
        match (open, begin) {
            (Some(name), Some(_)) => return Err(format!("region {name} is not closed")),
            (Some(_), None) if text == END => {
                out.push_str(line);
                open = None;
            }
            (Some(_), None) => {}
            (None, Some(name)) => {
                let Some((_, render)) = REGIONS.iter().find(|(region, _)| *region == name) else {
                    return Err(format!("unknown region {name}"));
                };
                if written.contains(&name) {
                    return Err(format!("region {name} is written twice"));
                }
                // Blank lines, so that a table is not taken to run into the
                // comments.
                let _ = write!(out, "{line}\n{}\n", render());
                written.push(name);
                open = Some(name);
            }
            (None, None) if text == END => return Err("a comment closes no region".to_owned()),
            (None, None) => out.push_str(line),
        }
    }
    if let Some(name) = open {
        return Err(format!("region {name} is not closed"));
    }
    match REGIONS.iter().find(|(name, _)| !written.contains(name)) {
        Some((name, _)) => Err(format!("no region {name}")),
        None => Ok(out),
    }
}

fn item_table<T>(settings: &[Setting<T>], part: fn(&Config) -> &T) -> String {
    let mut out = "| Item | Value | Default | Meaning |\n|---|---|---|---|\n".to_owned();
    for setting in settings {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} |",
            setting.key,
            (setting.expected)(),
            default(setting, part),
            setting.description
        );
    }
    out
}

// Without the column of values, which would say the same in every row.
fn color_table() -> String {
    let mut out = "| Item | Light | Dark | Used for |\n|---|---|---|---|\n".to_owned();
    for setting in COLORS {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} |",
            setting.key,
            default(setting, |config| &config.colors.light),
            default(setting, |config| &config.colors.dark),
            setting.description
        );
    }
    out
}

fn default<T>(setting: &Setting<T>, part: fn(&Config) -> &T) -> String {
    let [windows, macos, linux] = OSES.map(|(os, name)| {
        let value = (setting.write)(part(&Config::default_for(os)))
            .unwrap_or_else(|| panic!("{} should have a default to show", setting.key));
        (format!("`{value}`"), name)
    });
    if windows.0 == macos.0 && macos.0 == linux.0 {
        windows.0
    } else {
        format!(
            "{} on {}<br>{} on {}<br>{} on {}",
            windows.0, windows.1, macos.0, macos.1, linux.0, linux.1
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::config::parse_config;

    #[test]
    fn only_what_stands_between_the_comments_is_rewritten() {
        let current = skeleton().replace(
            "<!-- generated:log -->\n",
            "before\n<!-- generated:log -->\nstale\n",
        ) + "after\n";

        let doc = configuration_doc(&current).unwrap();

        assert!(doc.contains(&format!(
            "before\n<!-- generated:log -->\n\n{}\n{END}\n",
            item_table(LOG, |config| &config.log)
        )));
        assert!(doc.ends_with(&format!("{END}\nafter\n")));
        assert_eq!(configuration_doc(&doc).unwrap(), doc);
    }

    #[test]
    fn comments_that_do_not_pair_with_the_regions_are_errors() {
        let begin = "<!-- generated:log -->\n";
        let cases = [
            (
                skeleton() + "<!-- generated:nothing -->\n",
                "unknown region nothing",
            ),
            (skeleton() + begin, "region log is written twice"),
            (skeleton().replace(begin, ""), "a comment closes no region"),
            (
                skeleton().replace(&format!("{begin}{END}\n"), ""),
                "no region log",
            ),
            (
                skeleton().replace(&format!("{begin}{END}\n"), begin),
                "region log is not closed",
            ),
            (
                skeleton()
                    .strip_suffix(&format!("{END}\n"))
                    .unwrap()
                    .to_owned(),
                "region colors.themes is not closed",
            ),
        ];

        for (current, error) in cases {
            assert_eq!(configuration_doc(&current), Err(error.to_owned()));
        }
    }

    #[test]
    fn the_defaults_shown_read_back_as_the_defaults() {
        for (os, _) in OSES {
            let config = Config::default_for(os);
            let text = [
                table("filelist", FILELIST, &config.filelist),
                table("preview.text", TEXT_PREVIEW, &config.preview.text),
                table("preview.image", IMAGE_PREVIEW, &config.preview.image),
                table("log", LOG, &config.log),
                table("programs", PROGRAMS, &config.programs),
                table("font", FONT, &config.font.default),
                table("colors", COLORS_CONFIG, &config.colors),
                table("colors.light", COLORS, &config.colors.light),
                table("colors.dark", COLORS, &config.colors.dark),
            ]
            .concat();

            let (read, diagnostics) =
                parse_config([(Path::new("config.toml"), text.as_str())], os).unwrap();

            assert_eq!(diagnostics, [], "{os:?}");
            assert_eq!(read, config, "{os:?}");
        }
    }

    fn table<T>(header: &str, settings: &[Setting<T>], part: &T) -> String {
        let mut out = format!("[{header}]\n");
        for setting in settings {
            let value = (setting.write)(part).unwrap();
            let _ = writeln!(out, "{} = {value}", setting.key);
        }
        out
    }

    fn skeleton() -> String {
        let mut text = String::new();
        for (name, _) in REGIONS {
            let _ = writeln!(text, "<!-- generated:{name} -->\n{END}");
        }
        text
    }
}
