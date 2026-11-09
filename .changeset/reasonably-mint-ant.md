---
wafu: minor
---

On macOS and Linux, programs started by external commands and the editor now get the environment of your login shell (such as `PATH`) when wafu is not started from a terminal. Shell startup files can check `WAFU_RESOLVING_ENVIRONMENT` to skip lines that should not run while wafu reads it.
