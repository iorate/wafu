---
wafu: minor
---

Command lines in the configuration (editor and [commands]) are now split by the OS's own quoting rules, so some existing lines may need rewriting. On macOS and Linux, backslashes, single quotes and a leading # work as in sh; on Windows, a backslash before a double quote escapes it (write "C:\dir\\" for a path ending in a backslash), and a line starting with a space has no program and is rejected.
