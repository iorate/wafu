---
wafu: patch
---

A new file, a new text file, or the archive to compress to must now be given a single name, and `mkdir` makes directories only below the current one. Extracting into directories now fails for an archive whose name cannot be a directory there (such as `...zip`), and on Windows a name that Windows does not allow is refused with the reason instead of being tried.
