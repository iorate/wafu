use std::cmp::Ordering;

use crate::entry::Entry;

/// The entries of a location, with the display order applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    entries: Vec<Entry>,
    visible: Vec<usize>,
}

/// The sort settings of a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortSpec {
    pub key: SortKey,
    pub descending: bool,
    pub dirs_first: bool,
    pub ignore_case: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    None,
    Name,
    Extension,
    Size,
    Time,
    Natural,
}

impl Listing {
    #[must_use]
    pub fn new(entries: Vec<Entry>, sort: SortSpec, show_hidden: bool) -> Self {
        let mut listing = Self {
            entries,
            visible: Vec::new(),
        };
        listing.rebuild(sort, show_hidden);
        listing
    }

    pub fn rebuild(&mut self, sort: SortSpec, show_hidden: bool) {
        self.visible = (0..self.entries.len())
            .filter(|&index| show_hidden || !self.entries[index].hidden)
            .collect();
        let entries = &self.entries;
        self.visible
            .sort_by(|&a, &b| compare(sort, (a, &entries[a]), (b, &entries[b])));
    }

    #[must_use]
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    #[must_use]
    pub fn visible_entry(&self, index: usize) -> Option<&Entry> {
        self.visible.get(index).map(|&index| &self.entries[index])
    }

    pub fn visible_entries(&self) -> impl Iterator<Item = &Entry> {
        self.visible.iter().map(|&index| &self.entries[index])
    }
}

impl SortKey {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Name => "name",
            Self::Extension => "extension",
            Self::Size => "size",
            Self::Time => "time",
            Self::Natural => "natural",
        }
    }
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            descending: false,
            dirs_first: true,
            ignore_case: true,
        }
    }
}

fn compare(
    sort: SortSpec,
    (index_a, a): (usize, &Entry),
    (index_b, b): (usize, &Entry),
) -> Ordering {
    if sort.dirs_first {
        let order = b.kind.is_dir_like().cmp(&a.kind.is_dir_like());
        if order != Ordering::Equal {
            return order;
        }
    }
    let order = match sort.key {
        SortKey::None => return reverse_if(index_a.cmp(&index_b), sort.descending),
        SortKey::Name | SortKey::Natural => compare_names(a, b, sort.ignore_case),
        SortKey::Extension => compare_text(&a.split_name().1, &b.split_name().1, sort.ignore_case),
        SortKey::Size => sort_size(a).cmp(&sort_size(b)),
        SortKey::Time => a.modified.map(|t| t.at).cmp(&b.modified.map(|t| t.at)),
    };
    // Ties are broken by ascending name regardless of the direction.
    reverse_if(order, sort.descending).then_with(|| compare_names(a, b, sort.ignore_case))
}

fn compare_names(a: &Entry, b: &Entry, ignore_case: bool) -> Ordering {
    compare_text(
        &a.name.to_string_lossy(),
        &b.name.to_string_lossy(),
        ignore_case,
    )
    .then_with(|| a.name.cmp(&b.name))
}

fn compare_text(a: &str, b: &str, ignore_case: bool) -> Ordering {
    if ignore_case {
        a.to_lowercase().cmp(&b.to_lowercase())
    } else {
        a.cmp(b)
    }
}

fn sort_size(entry: &Entry) -> u64 {
    if entry.kind.is_dir_like() {
        0
    } else {
        entry.size
    }
}

fn reverse_if(order: Ordering, condition: bool) -> Ordering {
    if condition { order.reverse() } else { order }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::entry::{EntryKind, LocalDateTime, TargetKind, Timestamp};

    #[test]
    fn default_sort_puts_directories_first_and_ignores_case() {
        let listing = listing(
            &[
                file("b.txt", 0),
                dir("Z"),
                file("A.txt", 0),
                dir("a"),
                file("c.txt", 0),
            ],
            SortSpec::default(),
        );

        assert_eq!(names(&listing), ["a", "Z", "A.txt", "b.txt", "c.txt"]);
    }

    #[test]
    fn symlink_to_directory_is_grouped_with_directories() {
        let link = Entry {
            kind: EntryKind::Symlink {
                target: Some(TargetKind::Dir),
            },
            ..file("link", 0)
        };
        let listing = listing(&[file("a", 0), link, dir("z")], SortSpec::default());

        assert_eq!(names(&listing), ["link", "z", "a"]);
    }

    #[test]
    fn case_sensitive_sort_orders_by_code_point() {
        let sort = SortSpec {
            ignore_case: false,
            ..SortSpec::default()
        };
        let listing = listing(&[file("b", 0), file("A", 0), file("a", 0)], sort);

        assert_eq!(names(&listing), ["A", "a", "b"]);
    }

    #[test]
    fn descending_reverses_key_but_keeps_directories_first() {
        let sort = SortSpec {
            descending: true,
            ..SortSpec::default()
        };
        let listing = listing(&[file("a", 0), dir("d1"), file("b", 0), dir("d2")], sort);

        assert_eq!(names(&listing), ["d2", "d1", "b", "a"]);
    }

    #[test]
    fn dirs_first_off_mixes_directories_and_files() {
        let sort = SortSpec {
            dirs_first: false,
            ..SortSpec::default()
        };
        let listing = listing(&[file("a", 0), dir("c"), file("b", 0)], sort);

        assert_eq!(names(&listing), ["a", "b", "c"]);
    }

    #[test]
    fn extension_sort_breaks_ties_by_name() {
        let sort = SortSpec {
            key: SortKey::Extension,
            ..SortSpec::default()
        };
        let listing = listing(
            &[
                file("b.txt", 0),
                file("a.zip", 0),
                file("a.txt", 0),
                file("noext", 0),
            ],
            sort,
        );

        assert_eq!(names(&listing), ["noext", "a.txt", "b.txt", "a.zip"]);
    }

    #[test]
    fn size_sort_treats_directories_as_empty_and_breaks_ties_by_name() {
        let sort = SortSpec {
            key: SortKey::Size,
            dirs_first: false,
            ..SortSpec::default()
        };
        let big_dir = Entry {
            size: 4096,
            ..dir("d")
        };
        let listing = listing(&[file("b", 10), file("a", 10), big_dir, file("c", 1)], sort);

        assert_eq!(names(&listing), ["d", "c", "a", "b"]);
    }

    #[test]
    fn size_sort_descending_breaks_ties_by_ascending_name() {
        let sort = SortSpec {
            key: SortKey::Size,
            descending: true,
            ..SortSpec::default()
        };
        let listing = listing(&[file("b", 10), file("a", 10), file("c", 1)], sort);

        assert_eq!(names(&listing), ["a", "b", "c"]);
    }

    #[test]
    fn time_sort_puts_unknown_time_first() {
        let sort = SortSpec {
            key: SortKey::Time,
            ..SortSpec::default()
        };
        let listing = listing(
            &[timed("new", 200), timed("old", 100), file("unknown", 0)],
            sort,
        );

        assert_eq!(names(&listing), ["unknown", "old", "new"]);
    }

    #[test]
    fn none_sort_keeps_read_order() {
        let sort = SortSpec {
            key: SortKey::None,
            ..SortSpec::default()
        };
        let listing = listing(&[file("c", 0), dir("z"), file("a", 0), dir("y")], sort);

        assert_eq!(names(&listing), ["z", "y", "c", "a"]);
    }

    #[test]
    fn none_sort_descending_reverses_read_order() {
        let sort = SortSpec {
            key: SortKey::None,
            descending: true,
            dirs_first: false,
            ..SortSpec::default()
        };
        let listing = listing(&[file("c", 0), file("a", 0), file("b", 0)], sort);

        assert_eq!(names(&listing), ["b", "a", "c"]);
    }

    #[test]
    fn hidden_entries_are_excluded_unless_shown() {
        let entries = [file("a", 0), hidden(".b")];

        let shown = Listing::new(entries.to_vec(), SortSpec::default(), true);
        let not_shown = Listing::new(entries.to_vec(), SortSpec::default(), false);

        assert_eq!(names(&shown), [".b", "a"]);
        assert_eq!(names(&not_shown), ["a"]);
    }

    fn listing(entries: &[Entry], sort: SortSpec) -> Listing {
        Listing::new(entries.to_vec(), sort, true)
    }

    fn names(listing: &Listing) -> Vec<String> {
        listing
            .visible_entries()
            .map(|entry| entry.name.to_string_lossy().into_owned())
            .collect()
    }

    fn file(name: &str, size: u64) -> Entry {
        Entry {
            name: OsString::from(name),
            kind: EntryKind::File,
            size,
            modified: None,
            hidden: false,
        }
    }

    fn dir(name: &str) -> Entry {
        Entry {
            kind: EntryKind::Dir,
            ..file(name, 0)
        }
    }

    fn hidden(name: &str) -> Entry {
        Entry {
            hidden: true,
            ..file(name, 0)
        }
    }

    fn timed(name: &str, seconds: u64) -> Entry {
        Entry {
            modified: Some(Timestamp {
                at: SystemTime::UNIX_EPOCH + Duration::from_secs(seconds),
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
