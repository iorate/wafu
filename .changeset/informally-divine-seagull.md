---
wafu: minor
---

Commands pick their targets by one rule: open, open_with_os, enter_as_archive, edit, and rename act on the cursor row whatever is marked, while copy, move, delete, trash, extract, compress, copy_path, copy_name, shell_menu, and the {paths} of external commands act on the marks or, without marks, on the cursor row; a right click follows the same rule instead of singling out an unmarked row. The .. row now stands for the directory itself (or the archive file at an archive root) for every command, so copy_path, compress, rename, and the rest work on it as open_with_os already did, and renaming it moves the pane to the new name.
