---
wafu: patch
---

Extracting a tar onto a file system that ignores case no longer gives a hard link the wrong content when a later entry overwrites the file it links to under another spelling of its name.
