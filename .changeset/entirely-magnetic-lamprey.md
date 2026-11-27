---
wafu: minor
---

A command line can now use only one of `{cursor_path}`, `{marked_paths}`, and `{paths}`; a line that mixes them is an error. Inside an archive, a command extracts only the items its line uses, and a line with `{cursor_path}` now starts on a `..` row while items are marked.
