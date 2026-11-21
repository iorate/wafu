---
wafu: patch
---

Renaming an item to a name that differs only in case no longer replaces another item of that name. On a case-sensitive file system, renaming a symbolic link onto its target lost the target, and renaming a hard link onto another reported success without doing anything; both now fail with "already exists".
