---
wafu: minor
---

Placeholders in command lines are now replaced anywhere in an argument (`--dir={dir}`), an argument that holds `{paths}` or `{marked_paths}` is repeated for each path, a brace meant as text must be doubled (`{{`, `}}`) while any other brace or unknown name is an error, and a command whose placeholder has no value is not started. `editor` and `[open_alternative]` now take `{path}`, the file they open, as their only placeholder, in place of `{paths}` and `{cursor_path}`.

The note of `rename_template` now quotes an unknown placeholder, as the log does: `Unknown placeholder "{x}"`.
