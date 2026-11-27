---
wafu: minor
---

Placeholders in `editor`, `[commands]`, `[open_alternative]`, and the `run` dialogs are now replaced anywhere in an argument, so `-o{other_dir}` and `--dir={dir}` work, and an argument that holds `{paths}` or `{marked_paths}` is repeated for each path. A brace that is meant as text must be doubled (`{{`, `}}`); any other brace, an unknown name in braces, and a placeholder in the program are errors. A command whose placeholder has no value, such as `{marked_paths}` with nothing marked, is no longer started. In `editor`, `{cursor_path}` is now the file to edit for `edit_config` and `new_text` as well.
