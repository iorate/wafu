---
wafu: patch
---

Extracting an archive no longer changes the permissions or modification time of directories that already existed at the destination, and the permissions of extracted files and directories now respect the umask.
