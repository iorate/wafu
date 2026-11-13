---
wafu: minor
---

The configuration tables are reorganized: `[general]` is split into `[filelist]` (where `mkdir_then_enter` is now `mkdir_enters_dir`) and `[programs]` (`editor`, `use_login_shell_env`), `[appearance.font]` is now `[font]`, `theme` moves to `[colors]` and `size_format` to `[filelist]`, and `cursor-inactive` is now `cursor_inactive`. Items under the old names are reported as unknown and ignored.
