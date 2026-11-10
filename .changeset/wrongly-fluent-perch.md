---
wafu: minor
---

Command lines in the configuration (editor and [commands]) are now split by the OS's own quoting rules, so an existing line may need rewriting. On macOS and Linux a backslash, a single quote, and a # at the start of a word now work as in sh; on Windows, in an argument, a backslash before a double quote escapes it (write "C:\dir\\" for a path ending in a backslash) and "" inside quotes is a double quote, and a line starting with a space or tab is rejected as having no program.
