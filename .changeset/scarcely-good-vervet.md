---
wafu: patch
---

Fix compressing over one of the files being compressed deleting that file. Renaming the output onto a source in the conflict popup, or a default name that differs from a source only in case, now fails instead.
