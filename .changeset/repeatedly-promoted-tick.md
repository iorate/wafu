---
wafu: patch
---

Fix `edit`, `open_with_os`, `run`, and external commands following a symbolic link inside an archive to the file it points to outside the archive. They now do nothing for such a link and log the reason.
