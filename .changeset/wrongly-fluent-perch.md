---
wafu: minor
---

Command lines in the configuration (editor and [commands]) are now split by the OS's own quoting rules. On macOS and Linux a backslash, a single quote, and a # at the start of a word now work as in sh, and on Windows a backslash before a double quote now escapes it (write "C:\dir\\" for a path ending in a backslash), so an existing line using them may need rewriting.
