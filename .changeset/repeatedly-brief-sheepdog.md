---
wafu: patch
---

Skip symbolic links when extracting a directory inside an archive for another program (`open_with_os`, `edit`, `run_file`, external commands). The program is not started, since a link could lead it to a file outside the archive.
