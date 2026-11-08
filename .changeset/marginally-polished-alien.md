---
wafu: patch
---

Extracting a zip whose entries start with `./`, as one made from `.` does, no longer fails on its root entry; when a zip holds the same file under two spellings, such as `a.txt` and `./a.txt`, only the later one is listed and extracted.
