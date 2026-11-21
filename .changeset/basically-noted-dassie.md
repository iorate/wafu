---
wafu: patch
---

`rename`, and renaming in a copy or move conflict, now refuse a name that is `.` or `..` or contains a path separator, instead of moving the item to another directory.
