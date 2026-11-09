---
wafu: minor
---

On macOS, programs started by external commands and the editor now get the environment of your login shell (such as `PATH`) when wafu is launched from the Finder or the Dock. Shell startup files can check `WAFU_RESOLVING_ENVIRONMENT` to skip lines that should not run while wafu reads it.
