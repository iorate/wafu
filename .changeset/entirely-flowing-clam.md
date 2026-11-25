---
wafu: patch
---

Opening a tar archive no longer reads a long name, long link target, or PAX header of any declared size into memory. An archive with one larger than 1 MiB is reported as invalid instead of crashing or freezing wafu.
