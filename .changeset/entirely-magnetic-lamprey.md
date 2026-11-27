---
wafu: minor
---

A command line can now use only one of `{cursor_path}`, `{marked_paths}`, and `{paths}`; a line that mixes them is an error. A command now checks only the items its line uses: an item it does not use no longer keeps it from starting because of a name ending with a dot or a space on Windows, or because it is a link inside an archive. Inside an archive, a command extracts only those items, and a line with `{cursor_path}` now starts on a `..` row while items are marked.
