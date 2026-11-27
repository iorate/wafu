---
wafu: minor
---

Command names are made consistent, which gives twenty commands and one setting new names. Key bindings and settings under the old names are reported as unknown and ignored. The popup of `new_dir` is now titled `New directory`, and the popup of `preview_encodings` is titled `Encodings`. A command line that cannot be parsed in the popup of `run` or `run_command_line` is logged as `Cannot run command line: …`, without the command name.

| Old name | New name |
|---|---|
| `enter_as_archive` | `open_as_archive` |
| `history_back` | `go_back` |
| `history_forward` | `go_forward` |
| `drive_prev` | `go_prev_drive` |
| `drive_next` | `go_next_drive` |
| `switch_pane` | `focus_other` |
| `sync_other_to_this` | `sync_from_other` |
| `page_up` | `cursor_page_up` |
| `page_down` | `cursor_page_down` |
| `next_mark` | `cursor_next_mark` |
| `prev_mark` | `cursor_prev_mark` |
| `pane_shrink_left` | `split_left` |
| `pane_grow_left` | `split_right` |
| `clear_log` | `log_clear` |
| `copy_log` | `log_copy` |
| `preview_encoding` | `preview_encodings` |
| `close_preview` | `preview_close` |
| `mkdir` | `new_dir` |
| `run_file` | `run` |
| `run_command` | `run_command_line` |
| `mkdir_enters_dir` (setting) | `new_dir_enters_dir` |
