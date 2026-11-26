---
wafu: patch
---

Extracting a tar no longer gives a hard link the wrong content when a later entry overwrites the file it links to under another spelling of its name, as `a` does to `A` on macOS and Windows.
