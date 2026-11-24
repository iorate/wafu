---
wafu: patch
---

Fix overwriting a file or link with itself deleting it when it cannot be read, and a FIFO freezing an overwrite or a rename. Moving a directory onto itself through a link or another spelling of the path now fails instead of removing its empty directories.
