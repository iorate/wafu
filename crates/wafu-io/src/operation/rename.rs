use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use wafu_core::{OperationEvent, Renames, nfc};

use super::{Stop, Worker, fail};
use crate::fs_ops::{already_exists, is_taken, rename};

impl Worker<'_> {
    pub(super) fn rename_all(&mut self, renames: &Renames) -> Result<(), Stop> {
        let blockers = blockers(&renames.pairs);
        check(renames, &blockers)?;
        let taken = new_names(renames);
        for step in plan(&blockers) {
            self.check_cancel()?;
            match step {
                Step::One(index) => self.rename_one(renames, index)?,
                Step::Cycle(cycle) => self.rename_cycle(renames, &taken, &cycle)?,
            }
        }
        Ok(())
    }

    fn rename_one(&mut self, renames: &Renames, index: usize) -> Result<(), Stop> {
        let (old, new) = &renames.pairs[index];
        self.rename_entry(&renames.dir.join(old), &renames.dir.join(new))?;
        self.done += 1;
        (self.report)(OperationEvent::ItemDone { index });
        Ok(())
    }

    // A cancellation waits for the end of the cycle, as stopping in between
    // would leave the first item under the temporary name.
    fn rename_cycle(
        &mut self,
        renames: &Renames,
        taken: &HashSet<String>,
        cycle: &[usize],
    ) -> Result<(), Stop> {
        let first = cycle[0];
        let temp = temp_name(&renames.dir, taken);
        let temp_path = renames.dir.join(&temp);
        self.rename_entry(&renames.dir.join(&renames.pairs[first].0), &temp_path)?;
        if let Err(stop) = self.rename_rest(renames, cycle, &temp_path) {
            (self.report)(OperationEvent::CycleBroken {
                cycle: cycle.to_vec(),
                left: first,
                temp,
            });
            return Err(stop);
        }
        for &index in cycle {
            (self.report)(OperationEvent::ItemDone { index });
        }
        Ok(())
    }

    fn rename_rest(
        &mut self,
        renames: &Renames,
        cycle: &[usize],
        temp_path: &Path,
    ) -> Result<(), Stop> {
        let dir = &renames.dir;
        for &index in cycle[1..].iter().rev() {
            let (old, new) = &renames.pairs[index];
            self.rename_entry(&dir.join(old), &dir.join(new))?;
            self.done += 1;
        }
        self.rename_entry(temp_path, &dir.join(&renames.pairs[cycle[0]].1))?;
        self.done += 1;
        Ok(())
    }

    fn rename_entry(&mut self, from: &Path, to: &Path) -> Result<(), Stop> {
        self.report_progress(from);
        rename(from, to).map_err(fail(from))
    }
}

enum Step {
    One(usize),
    Cycle(Vec<usize>),
}

fn blockers(pairs: &[(String, String)]) -> Vec<Option<usize>> {
    let olds: HashMap<Cow<'_, str>, usize> = pairs
        .iter()
        .enumerate()
        .map(|(index, (old, _))| (nfc(old), index))
        .collect();
    pairs
        .iter()
        .enumerate()
        .map(|(index, (_, new))| {
            olds.get(&nfc(new))
                .copied()
                .filter(|&blocker| blocker != index)
        })
        .collect()
}

fn check(renames: &Renames, blockers: &[Option<usize>]) -> Result<(), Stop> {
    for ((old, new), blocker) in renames.pairs.iter().zip(blockers) {
        let from = renames.dir.join(old);
        fs::symlink_metadata(&from).map_err(fail(&from))?;
        if blocker.is_none() && is_taken(&from, &renames.dir.join(new)) {
            return Err(Stop::Failed {
                path: from,
                error: already_exists(),
            });
        }
    }
    Ok(())
}

fn plan(blockers: &[Option<usize>]) -> Vec<Step> {
    let mut reached_from: Vec<Option<usize>> = vec![None; blockers.len()];
    let mut steps = Vec::new();
    for start in 0..blockers.len() {
        let mut chain = Vec::new();
        let mut at = Some(start);
        while let Some(index) = at {
            match reached_from[index] {
                Some(from) if from == start => {
                    let position = chain
                        .iter()
                        .position(|&item| item == index)
                        .expect("an item reached from this start is on its chain");
                    steps.push(Step::Cycle(chain.split_off(position)));
                    break;
                }
                Some(_) => break,
                None => {
                    reached_from[index] = Some(start);
                    chain.push(index);
                    at = blockers[index];
                }
            }
        }
        steps.extend(chain.into_iter().rev().map(Step::One));
    }
    steps
}

// The temporary name stays off the new names, so that an item left under it
// after a failure does not sit where a rename not yet done wants to go. They
// are lowercased because a file system that ignores case sees names that
// differ only in case as the same.
fn new_names(renames: &Renames) -> HashSet<String> {
    renames
        .pairs
        .iter()
        .map(|(_, new)| nfc(new).to_lowercase())
        .collect()
}

fn temp_name(dir: &Path, taken: &HashSet<String>) -> String {
    let mut number: u64 = 0;
    loop {
        let name = format!("wafu-rename-{number}.tmp");
        if !taken.contains(&name) && fs::symlink_metadata(dir.join(&name)).is_err() {
            return name;
        }
        number += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc;

    use tempfile::TempDir;
    use wafu_core::{OperationKind, OperationSpec, Outcome, Reply, Sources};

    use super::*;
    use crate::archive::NestedArchives;
    use crate::operation::run_operation;
    use crate::watch::tests::symlink;

    #[test]
    fn a_missing_old_name_fails_before_anything_is_renamed() {
        let dir = files(&["a"]);

        let (outcome, _) = run(&dir, &[("a", "x"), ("missing", "y")], |_| None);

        let Outcome::Failed { path, .. } = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert_eq!(path, dir.path().join("missing"));
        assert_eq!(names(&dir), ["a"]);
    }

    #[test]
    fn a_taken_new_name_fails_before_anything_is_renamed() {
        let dir = files(&["a", "b", "c"]);

        let (outcome, _) = run(&dir, &[("a", "x"), ("b", "c")], |_| None);

        let Outcome::Failed { path, error } = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert_eq!(path, dir.path().join("b"));
        assert_eq!(error.to_string(), "already exists");
        assert_eq!(names(&dir), ["a", "b", "c"]);
    }

    #[test]
    fn a_link_at_a_new_name_is_taken_even_when_broken() {
        let dir = files(&["a", "b"]);
        if !symlink(Path::new("nowhere"), &dir.path().join("link")) {
            return;
        }

        let (outcome, _) = run(&dir, &[("a", "x"), ("b", "link")], |_| None);

        let Outcome::Failed { path, error } = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert_eq!(path, dir.path().join("b"));
        assert_eq!(error.to_string(), "already exists");
        assert!(dir.path().join("a").exists());
    }

    // On a file system that ignores case, the new name is taken by the item
    // itself; on one that does not, it is free.
    #[test]
    fn changing_only_the_case_passes_the_check() {
        let dir = files(&["a.txt"]);

        let (outcome, _) = run(&dir, &[("a.txt", "A.txt")], |_| None);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(names(&dir), ["A.txt"]);
    }

    #[test]
    fn a_chain_is_renamed_from_its_end_and_reported_by_position_in_the_list() {
        let dir = files(&["001", "002", "003"]);

        let (outcome, events) = run(
            &dir,
            &[("001", "002"), ("002", "003"), ("003", "004")],
            |_| None,
        );

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(content(&dir, "002"), "001");
        assert_eq!(content(&dir, "003"), "002");
        assert_eq!(content(&dir, "004"), "003");
        assert_eq!(names(&dir), ["002", "003", "004"]);
        assert_eq!(items_done(&events), [2, 1, 0]);
    }

    #[test]
    fn a_new_name_is_matched_to_an_old_name_regardless_of_normalization() {
        let dir = files(&["\u{304b}\u{3099}", "y"]);

        let (outcome, events) = run(
            &dir,
            &[("y", "\u{304c}"), ("\u{304b}\u{3099}", "x")],
            |_| None,
        );

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(items_done(&events), [1, 0]);
        assert_eq!(content(&dir, "x"), "\u{304b}\u{3099}");
    }

    #[test]
    fn a_cycle_is_resolved_through_a_temporary_name() {
        let dir = files(&["a", "b", "c"]);

        let (outcome, events) = run(&dir, &[("a", "b"), ("b", "c"), ("c", "a")], |_| None);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(content(&dir, "b"), "a");
        assert_eq!(content(&dir, "c"), "b");
        assert_eq!(content(&dir, "a"), "c");
        assert_eq!(names(&dir), ["a", "b", "c"]);
        let mut done = items_done(&events);
        done.sort_unstable();
        assert_eq!(done, [0, 1, 2]);
    }

    #[test]
    fn a_failure_in_a_cycle_reports_the_item_left_under_a_free_temporary_name() {
        let dir = files(&["a", "c", "wafu-rename-0.tmp"]);
        let renames = Renames {
            dir: dir.path().to_path_buf(),
            pairs: pairs(&[
                ("a", "b"),
                ("b", "c"),
                ("c", "a"),
                ("d", "wafu-rename-1.tmp"),
            ]),
        };
        let events = Mutex::new(Vec::new());
        let report = |event| events.lock().unwrap().push(event);
        let (_sender, replies) = mpsc::channel();
        let nested = NestedArchives::default();
        let mut worker = worker(&nested, &replies, &report);

        // `b` is missing, which the check before the start would catch. `c`
        // is renamed to `a` before `b` fails.
        let result = worker.rename_cycle(&renames, &new_names(&renames), &[0, 1, 2]);

        let Err(Stop::Failed { path, .. }) = result else {
            panic!("expected a failure");
        };
        assert_eq!(path, dir.path().join("b"));
        assert_eq!(content(&dir, "wafu-rename-2.tmp"), "a");
        assert_eq!(content(&dir, "a"), "c");
        let events = events.into_inner().unwrap();
        assert_eq!(items_done(&events), []);
        assert_eq!(
            events
                .into_iter()
                .filter(|event| matches!(event, OperationEvent::CycleBroken { .. }))
                .collect::<Vec<_>>(),
            [OperationEvent::CycleBroken {
                cycle: vec![0, 1, 2],
                left: 0,
                temp: "wafu-rename-2.tmp".to_owned(),
            }]
        );
    }

    #[test]
    fn a_failure_before_anything_in_a_cycle_is_renamed_reports_nothing_left() {
        let dir = files(&["b"]);
        let renames = Renames {
            dir: dir.path().to_path_buf(),
            pairs: pairs(&[("a", "b"), ("b", "a")]),
        };
        let events = Mutex::new(Vec::new());
        let report = |event| events.lock().unwrap().push(event);
        let (_sender, replies) = mpsc::channel();
        let nested = NestedArchives::default();
        let mut worker = worker(&nested, &replies, &report);

        let result = worker.rename_cycle(&renames, &new_names(&renames), &[0, 1]);

        assert!(matches!(result, Err(Stop::Failed { .. })));
        assert!(
            !events
                .into_inner()
                .unwrap()
                .iter()
                .any(|event| matches!(event, OperationEvent::CycleBroken { .. }))
        );
        assert_eq!(names(&dir), ["b"]);
    }

    #[test]
    fn a_failure_stops_with_the_items_before_it_left_renamed() {
        let dir = files(&["a", "b", "c"]);

        // The standard library refuses a path with a NUL before it reaches
        // the file system, so renaming to it fails.
        let (outcome, events) = run(&dir, &[("a", "x"), ("b", "y\0"), ("c", "z")], |_| None);

        let Outcome::Failed { path, .. } = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert_eq!(path, dir.path().join("b"));
        assert_eq!(names(&dir), ["b", "c", "x"]);
        assert_eq!(items_done(&events), [0]);
    }

    #[test]
    fn a_cancellation_is_taken_between_renames() {
        let dir = files(&["a", "b"]);

        let (outcome, events) = run(&dir, &[("a", "x"), ("b", "y")], |event| {
            matches!(event, OperationEvent::ItemDone { .. }).then_some(Reply::Cancel)
        });

        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(names(&dir), ["b", "x"]);
        assert_eq!(items_done(&events), [0]);
    }

    #[test]
    fn a_cancellation_during_a_cycle_waits_for_its_end() {
        let dir = files(&["a", "b", "c"]);

        let (outcome, _) = run(&dir, &[("a", "b"), ("b", "a"), ("c", "z")], |event| {
            matches!(event, OperationEvent::Progress { .. }).then_some(Reply::Cancel)
        });

        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(content(&dir, "a"), "b");
        assert_eq!(content(&dir, "b"), "a");
        assert_eq!(names(&dir), ["a", "b", "c"]);
    }

    #[test]
    fn a_cancellation_during_the_last_cycle_ends_in_success() {
        let dir = files(&["a", "b"]);

        let (outcome, _) = run(&dir, &[("a", "b"), ("b", "a")], |event| {
            matches!(event, OperationEvent::Progress { .. }).then_some(Reply::Cancel)
        });

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(content(&dir, "a"), "b");
    }

    fn run(
        dir: &TempDir,
        renames: &[(&str, &str)],
        react: impl Fn(&OperationEvent) -> Option<Reply>,
    ) -> (Outcome, Vec<OperationEvent>) {
        let (sender, replies) = mpsc::channel();
        let events = Mutex::new(Vec::new());
        let report = |event| {
            if let Some(reply) = react(&event) {
                sender.send(reply).unwrap();
            }
            events.lock().unwrap().push(event);
        };
        let spec = OperationSpec {
            kind: OperationKind::Rename,
            sources: Sources::Renames(Renames {
                dir: dir.path().to_path_buf(),
                pairs: pairs(renames),
            }),
            dest: None,
            output: None,
        };

        run_operation(&spec, None, &NestedArchives::default(), &replies, &report);

        let mut events = events.into_inner().unwrap();
        let Some(OperationEvent::Finished(outcome)) = events.pop() else {
            panic!("the rename did not finish");
        };
        (outcome, events)
    }

    fn worker<'a>(
        nested: &'a NestedArchives,
        replies: &'a mpsc::Receiver<Reply>,
        report: &'a dyn Fn(OperationEvent),
    ) -> Worker<'a> {
        Worker {
            owner_window: None,
            nested,
            replies,
            report,
            policy: None,
            done: 0,
            skipped: 0,
            last_progress: None,
        }
    }

    // Each file holds its own name.
    fn files(names: &[&str]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for name in names {
            fs::write(dir.path().join(name), name).unwrap();
        }
        dir
    }

    fn pairs(renames: &[(&str, &str)]) -> Vec<(String, String)> {
        renames
            .iter()
            .map(|&(old, new)| (old.to_owned(), new.to_owned()))
            .collect()
    }

    fn names(dir: &TempDir) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn content(dir: &TempDir, name: &str) -> String {
        fs::read_to_string(dir.path().join(name)).unwrap()
    }

    fn items_done(events: &[OperationEvent]) -> Vec<usize> {
        events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::ItemDone { index } => Some(*index),
                _ => None,
            })
            .collect()
    }
}
