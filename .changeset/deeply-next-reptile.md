---
wafu: minor
---

Command lines read their placeholders by new rules. A placeholder is now replaced anywhere in an argument, so `-o{other_dir}` and `--dir={dir}` work, and an argument that holds `{paths}` or `{marked_paths}` is repeated for each path. A brace that is meant as text must be doubled (`{{`, `}}`); any other brace, an unknown name in braces, and a placeholder in the program are errors. A command whose placeholder has no value, such as `{marked_paths}` with nothing marked, is no longer started. `editor` and `[open_alternative]` now take `{path}`, the file they open, as their only placeholder: replace `{paths}` and `{cursor_path}` there with `{path}`.
