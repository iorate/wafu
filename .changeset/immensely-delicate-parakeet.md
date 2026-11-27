---
wafu: minor
---

The names of configuration items are made consistent, which renames the items below. An item under an old name is reported as unknown and ignored; `[keys.image]` and `[keys.text]` are ignored as a whole, with every binding in them. A command table that still has `run` is reported as missing `command_line` and is left undefined, so the keys bound to it do nothing.

| Old name | New name |
|---|---|
| `[keys.image]` | `[keys.preview.image]` |
| `[keys.text]` | `[keys.preview.text]` |
| `mkdir_enters_dir` | `new_dir_goes_to_dir` |
| `search_matches_anywhere = true` / `false` | `search_match = "anywhere"` / `"start"` |
| `compare_tolerance_ms` | `mark_compare_tolerance_ms` |
| `max_find_results` | `max_find_matches` |
| `from_left`, `from_right` under `[preview.image]` | `placement_from_left`, `placement_from_right` |
| `fifo` under `[colors.light]` and `[colors.dark]` | `special` |
| `broken` under `[colors.light]` and `[colors.dark]` | `link_broken` |
| `run` in a command written as a table | `command_line` |
