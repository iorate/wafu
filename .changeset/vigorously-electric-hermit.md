---
wafu: patch
---

Compressing now fails at a file name or link target that is not valid UTF-8 instead of replacing the invalid characters, which could give two files the same name in the archive.
