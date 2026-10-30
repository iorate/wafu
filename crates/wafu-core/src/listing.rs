use std::cmp::Ordering;

use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::preferences::CollationNumericOrdering;
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};

use crate::entry::{Directory, Entry};
use crate::filter::Filter;

/// The entries of a location, with the display order applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    own: Option<Entry>,
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
    pub fn new(
        directory: Directory,
        sort: SortSpec,
        show_hidden: bool,
        filter: Option<&Filter>,
    ) -> Self {
        let mut listing = Self {
            own: directory.own,
            entries: directory.entries,
            visible: Vec::new(),
        };
        listing.rebuild(sort, show_hidden, filter);
        listing
    }

    /// Recomputes the visible rows. The filter applies to files only.
    pub fn rebuild(&mut self, sort: SortSpec, show_hidden: bool, filter: Option<&Filter>) {
        let collator = collator(sort);
        let mut items: Vec<(usize, Keys)> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| show_hidden || !entry.hidden)
            .filter(|(_, entry)| {
                entry.kind.is_dir_like() || filter.is_none_or(|f| f.matches(&entry.name))
            })
            .map(|(index, entry)| (index, Keys::new(&collator, entry, sort.key)))
            .collect();
        let entries = &self.entries;
        items.sort_by(|(a, ka), (b, kb)| {
            compare(sort, (*a, &entries[*a], ka), (*b, &entries[*b], kb))
        });
        self.visible = items.into_iter().map(|(index, _)| index).collect();
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

    /// The location itself, when the I/O layer could read it.
    #[must_use]
    pub fn own(&self) -> Option<&Entry> {
        self.own.as_ref()
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

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [
            Self::None,
            Self::Name,
            Self::Extension,
            Self::Size,
            Self::Time,
            Self::Natural,
        ]
        .into_iter()
        .find(|key| key.name() == name)
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

// Collation keys of one entry, computed once per rebuild because comparing
// strings through the collator on every comparison is several times slower.
struct Keys {
    name: Vec<u8>,
    extension: Vec<u8>,
}

impl Keys {
    fn new(collator: &CollatorBorrowed<'_>, entry: &Entry, key: SortKey) -> Self {
        let extension = if key == SortKey::Extension {
            sort_key(collator, &entry.split_name().1)
        } else {
            Vec::new()
        };
        Self {
            name: sort_key(collator, &entry.name.to_string_lossy()),
            extension,
        }
    }
}

// The root locale keeps the order the same on every machine. Numeric ordering
// is what makes "natural" sort compare digit runs by value.
fn collator(sort: SortSpec) -> CollatorBorrowed<'static> {
    let mut prefs = CollatorPreferences::default();
    prefs.numeric_ordering = Some(if sort.key == SortKey::Natural {
        CollationNumericOrdering::True
    } else {
        CollationNumericOrdering::False
    });
    let mut options = CollatorOptions::default();
    options.strength = Some(if sort.ignore_case {
        Strength::Secondary
    } else {
        Strength::Tertiary
    });
    Collator::try_new(prefs, options).expect("the compiled collation data should load")
}

fn sort_key(collator: &CollatorBorrowed<'_>, text: &str) -> Vec<u8> {
    let mut key = Vec::new();
    match collator.write_sort_key_to(text, &mut key) {
        Ok(()) => key,
        Err(never) => match never {},
    }
}

fn compare(
    sort: SortSpec,
    (index_a, a, keys_a): (usize, &Entry, &Keys),
    (index_b, b, keys_b): (usize, &Entry, &Keys),
) -> Ordering {
    if sort.dirs_first {
        let order = b.kind.is_dir_like().cmp(&a.kind.is_dir_like());
        if order != Ordering::Equal {
            return order;
        }
    }
    let order = match sort.key {
        SortKey::None => return reverse_if(index_a.cmp(&index_b), sort.descending),
        SortKey::Name | SortKey::Natural => compare_names((a, keys_a), (b, keys_b)),
        SortKey::Extension => keys_a.extension.cmp(&keys_b.extension),
        SortKey::Size => sort_size(a).cmp(&sort_size(b)),
        SortKey::Time => a.modified.map(|t| t.at).cmp(&b.modified.map(|t| t.at)),
    };
    // Ties are broken by ascending name regardless of the direction.
    reverse_if(order, sort.descending).then_with(|| compare_names((a, keys_a), (b, keys_b)))
}

fn compare_names((a, keys_a): (&Entry, &Keys), (b, keys_b): (&Entry, &Keys)) -> Ordering {
    keys_a
        .name
        .cmp(&keys_b.name)
        .then_with(|| a.name.cmp(&b.name))
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
    fn ignoring_case_breaks_ties_by_bytes() {
        let listing = listing(
            &[file("a", 0), file("A", 0), file("b", 0)],
            SortSpec::default(),
        );

        assert_eq!(names(&listing), ["A", "a", "b"]);
    }

    #[test]
    fn case_sensitive_sort_puts_lower_case_first() {
        let sort = SortSpec {
            ignore_case: false,
            ..SortSpec::default()
        };
        let listing = listing(&[file("b", 0), file("A", 0), file("a", 0)], sort);

        assert_eq!(names(&listing), ["a", "A", "b"]);
    }

    #[test]
    fn name_sort_follows_the_collation_order() {
        let listing = listing(
            &[
                file("b", 0),
                file("ä", 0),
                file("a", 0),
                file("い", 0),
                file("あ", 0),
                file("1", 0),
            ],
            SortSpec::default(),
        );

        assert_eq!(names(&listing), ["1", "a", "ä", "b", "あ", "い"]);
    }

    #[test]
    fn name_sort_compares_digits_as_characters() {
        let listing = listing(
            &[file("a10", 0), file("a2", 0), file("a1", 0)],
            SortSpec::default(),
        );

        assert_eq!(names(&listing), ["a1", "a10", "a2"]);
    }

    #[test]
    fn natural_sort_compares_digit_runs_by_value() {
        let sort = SortSpec {
            key: SortKey::Natural,
            ..SortSpec::default()
        };
        let listing = listing(
            &[
                file("a10", 0),
                file("a2", 0),
                file("a1", 0),
                file("a02", 0),
                file("b", 0),
            ],
            sort,
        );

        assert_eq!(names(&listing), ["a1", "a02", "a2", "a10", "b"]);
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

        let shown = Listing::new(directory(&entries), SortSpec::default(), true, None);
        let not_shown = Listing::new(directory(&entries), SortSpec::default(), false, None);

        assert_eq!(names(&shown), [".b", "a"]);
        assert_eq!(names(&not_shown), ["a"]);
    }

    #[test]
    fn filter_applies_to_files_only() {
        let filter = Filter::parse("*.png").unwrap().unwrap();
        let entries = [file("a.png", 0), file("b.txt", 0), dir("c.txt")];

        let listing = Listing::new(
            directory(&entries),
            SortSpec::default(),
            true,
            Some(&filter),
        );

        assert_eq!(names(&listing), ["c.txt", "a.png"]);
    }

    fn listing(entries: &[Entry], sort: SortSpec) -> Listing {
        Listing::new(directory(entries), sort, true, None)
    }

    fn directory(entries: &[Entry]) -> Directory {
        Directory {
            own: None,
            entries: entries.to_vec(),
        }
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
