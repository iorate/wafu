use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::time::Duration;

use crate::entry::{Entry, EntryKind, Timestamp};
use crate::text::nfc;

/// The settings of `mark_compare`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompareSpec {
    pub condition: CompareCondition,
    pub include_dirs: bool,
    pub ignore_case: bool,
}

impl Default for CompareSpec {
    fn default() -> Self {
        Self {
            condition: CompareCondition::Missing,
            include_dirs: true,
            ignore_case: true,
        }
    }
}

/// Which files `mark_compare` marks. Each takes in what the one before it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareCondition {
    Missing,
    Newer,
    Different,
}

impl CompareCondition {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Newer => "newer",
            Self::Different => "different",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Missing, Self::Newer, Self::Different]
            .into_iter()
            .find(|condition| condition.name() == name)
    }
}

pub(crate) fn names_to_mark<'a>(
    targets: impl IntoIterator<Item = &'a Entry>,
    others: &[Entry],
    spec: CompareSpec,
    tolerance: Duration,
) -> Vec<OsString> {
    let mut by_key: HashMap<OsString, Vec<&Entry>> = HashMap::new();
    for entry in others {
        by_key
            .entry(match_key(&entry.name, spec.ignore_case))
            .or_default()
            .push(entry);
    }
    targets
        .into_iter()
        .filter(|target| {
            let found = by_key
                .get(&match_key(&target.name, spec.ignore_case))
                .and_then(|candidates| {
                    candidates
                        .iter()
                        .find(|entry| entry.name == target.name)
                        .or_else(|| candidates.iter().min_by(|a, b| a.name.cmp(&b.name)))
                })
                .copied();
            marks(target, found, spec, tolerance)
        })
        .map(|target| target.name.clone())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Age {
    Same,
    Newer,
    Older,
    Unknown,
}

fn match_key(name: &OsStr, ignore_case: bool) -> OsString {
    // A lossy conversion would make two different invalid names equal.
    let Some(name) = name.to_str() else {
        return name.to_owned();
    };
    let name = nfc(name);
    if ignore_case {
        name.to_lowercase().into()
    } else {
        name.into_owned().into()
    }
}

fn marks(target: &Entry, found: Option<&Entry>, spec: CompareSpec, tolerance: Duration) -> bool {
    // Nothing recurses, so a directory is judged by whether it exists alone:
    // its own size and time say nothing about what is inside.
    if target.kind.is_dir_like() {
        return spec.include_dirs && found.is_none();
    }
    let Some(found) = found else {
        return true;
    };
    // `copy` skips a file whose destination is a directory, but asks before
    // overwriting a link to one, so only a real directory is left alone.
    if found.kind == EntryKind::Dir {
        return false;
    }
    let age = age(target.modified, found.modified, tolerance);
    match spec.condition {
        CompareCondition::Missing => false,
        CompareCondition::Newer => age == Age::Newer,
        CompareCondition::Different => age != Age::Same || target.size != found.size,
    }
}

fn age(target: Option<Timestamp>, found: Option<Timestamp>, tolerance: Duration) -> Age {
    let (Some(target), Some(found)) = (target, found) else {
        return Age::Unknown;
    };
    match target.at.duration_since(found.at) {
        Ok(ahead) if ahead > tolerance => Age::Newer,
        Err(behind) if behind.duration() > tolerance => Age::Older,
        _ => Age::Same,
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;
    use crate::entry::{LocalDateTime, SpecialKind, TargetKind};

    const TOLERANCE: Duration = Duration::from_secs(2);
    const BASE_MS: u64 = 1_000_000;

    #[test]
    fn a_file_missing_on_the_other_side_is_marked_under_every_condition() {
        for condition in conditions() {
            let spec = spec(condition);

            assert!(
                marked(&timed("a", 0), &[timed("b", 0)], spec),
                "{condition:?}"
            );
            assert!(
                !marked(&timed("a", 0), &[timed("a", 0)], spec),
                "{condition:?}"
            );
        }
    }

    #[test]
    fn times_within_the_tolerance_are_the_same() {
        let cases = [
            (2000, false, false),
            (2001, true, true),
            (-2000, false, false),
            (-2001, false, true),
        ];
        for (offset, newer, different) in cases {
            let target = timed("a", offset);
            let others = [timed("a", 0)];

            assert_eq!(
                marked(&target, &others, spec(CompareCondition::Newer)),
                newer,
                "{offset}"
            );
            assert_eq!(
                marked(&target, &others, spec(CompareCondition::Different)),
                different,
                "{offset}"
            );
        }
    }

    #[test]
    fn an_unknown_time_is_not_newer_but_is_different() {
        for (target, found) in [
            (file("a", 0), timed("a", 0)),
            (timed("a", 0), file("a", 0)),
            (file("a", 0), file("a", 0)),
        ] {
            let others = [found];

            assert!(!marked(&target, &others, spec(CompareCondition::Newer)));
            assert!(marked(&target, &others, spec(CompareCondition::Different)));
        }
    }

    #[test]
    fn a_different_size_counts_only_for_different() {
        let target = Entry {
            size: 1,
            ..timed("a", 0)
        };
        let others = [timed("a", 0)];

        assert!(!marked(&target, &others, spec(CompareCondition::Newer)));
        assert!(marked(&target, &others, spec(CompareCondition::Different)));
    }

    #[test]
    fn a_file_is_never_marked_over_a_directory_of_its_name() {
        let target = Entry {
            size: 1,
            ..timed("a", 10_000)
        };
        let others = [dir("a")];

        for condition in conditions() {
            assert!(!marked(&target, &others, spec(condition)), "{condition:?}");
        }
    }

    #[test]
    fn a_link_to_a_directory_on_the_other_side_is_compared_as_an_item() {
        let others = [Entry {
            kind: EntryKind::Symlink {
                target: Some(TargetKind::Dir),
            },
            ..timed("a", 0)
        }];

        assert!(!marked(
            &timed("a", 0),
            &others,
            spec(CompareCondition::Missing)
        ));
        assert!(!marked(
            &timed("a", 0),
            &others,
            spec(CompareCondition::Different)
        ));
        assert!(marked(
            &timed("a", 10_000),
            &others,
            spec(CompareCondition::Newer)
        ));
    }

    #[test]
    fn links_and_special_files_are_targets_like_files() {
        let targets = [
            Entry {
                kind: EntryKind::Symlink {
                    target: Some(TargetKind::File { executable: false }),
                },
                ..timed("file_link", 10_000)
            },
            Entry {
                kind: EntryKind::Symlink { target: None },
                ..file("broken", 0)
            },
            Entry {
                kind: EntryKind::Special(SpecialKind::Fifo),
                ..file("fifo", 0)
            },
        ];
        let others = [timed("file_link", 0)];

        let names = names_to_mark(&targets, &others, spec(CompareCondition::Newer), TOLERANCE);

        assert_eq!(names, ["file_link", "broken", "fifo"].map(OsString::from));
    }

    #[test]
    fn directories_are_marked_only_when_missing_and_included() {
        let link = Entry {
            kind: EntryKind::Symlink {
                target: Some(TargetKind::Dir),
            },
            ..dir("link")
        };
        for target in [dir("d"), link] {
            let name = target.name.to_string_lossy().into_owned();
            let excluded = CompareSpec {
                include_dirs: false,
                ..spec(CompareCondition::Different)
            };

            assert!(!marked(&target, &[], excluded), "{name}");
            for condition in conditions() {
                let spec = spec(condition);
                assert!(marked(&target, &[], spec), "{name} {condition:?}");
                assert!(
                    !marked(&target, &[file(&name, 1)], spec),
                    "{name} {condition:?}"
                );
                let other_dir = Entry {
                    kind: EntryKind::Dir,
                    size: 1,
                    ..timed(&name, 10_000)
                };
                assert!(!marked(&target, &[other_dir], spec), "{name} {condition:?}");
            }
        }
    }

    #[test]
    fn names_match_in_composed_form() {
        let decomposed = timed("e\u{301}", 0);
        let others = [timed("\u{e9}", 0)];

        assert!(!marked(
            &decomposed,
            &others,
            spec(CompareCondition::Missing)
        ));
    }

    #[test]
    fn case_is_ignored_only_when_asked() {
        let others = [timed("A.TXT", 0)];
        let sensitive = CompareSpec {
            ignore_case: false,
            ..spec(CompareCondition::Missing)
        };

        assert!(!marked(
            &timed("a.txt", 0),
            &others,
            spec(CompareCondition::Missing)
        ));
        assert!(marked(&timed("a.txt", 0), &others, sensitive));
    }

    #[test]
    fn of_several_candidates_the_exact_name_is_compared() {
        let others = [timed("A.TXT", -10_000), timed("a.txt", 0)];

        assert!(!marked(
            &timed("a.txt", 0),
            &others,
            spec(CompareCondition::Newer)
        ));
    }

    #[test]
    fn without_the_exact_name_the_first_candidate_in_byte_order_is_compared() {
        let others = [
            timed("b.TXT", -10_000),
            timed("B.txt", 0),
            timed("b.Txt", -10_000),
        ];

        assert!(!marked(
            &timed("b.txt", 0),
            &others,
            spec(CompareCondition::Newer)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn names_that_are_not_valid_unicode_match_only_themselves() {
        use std::os::unix::ffi::OsStringExt;

        let invalid = |last: u8| Entry {
            name: OsString::from_vec(vec![b'a', last]),
            ..file("", 0)
        };

        assert!(marked(
            &invalid(0xff),
            &[invalid(0xfe)],
            spec(CompareCondition::Missing)
        ));
        assert!(!marked(
            &invalid(0xff),
            &[invalid(0xff)],
            spec(CompareCondition::Missing)
        ));
    }

    fn marked(target: &Entry, others: &[Entry], spec: CompareSpec) -> bool {
        !names_to_mark([target], others, spec, TOLERANCE).is_empty()
    }

    fn spec(condition: CompareCondition) -> CompareSpec {
        CompareSpec {
            condition,
            ..CompareSpec::default()
        }
    }

    fn conditions() -> [CompareCondition; 3] {
        [
            CompareCondition::Missing,
            CompareCondition::Newer,
            CompareCondition::Different,
        ]
    }

    fn file(name: &str, size: u64) -> Entry {
        Entry {
            name: OsString::from(name),
            kind: EntryKind::File { executable: false },
            size,
            modified: None,
            hidden: false,
            permissions: None,
            link_target: None,
        }
    }

    fn dir(name: &str) -> Entry {
        Entry {
            kind: EntryKind::Dir,
            ..file(name, 0)
        }
    }

    fn timed(name: &str, offset_ms: i64) -> Entry {
        let at = SystemTime::UNIX_EPOCH
            + Duration::from_millis(BASE_MS.saturating_add_signed(offset_ms));
        Entry {
            modified: Some(Timestamp {
                at,
                local: LocalDateTime {
                    year: 1970,
                    month: 1,
                    day: 1,
                    hour: 0,
                    minute: 0,
                    second: 0,
                },
            }),
            ..file(name, 0)
        }
    }
}
