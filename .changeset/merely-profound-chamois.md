---
wafu: patch
---

Fix a pane retrying without end, or starting with no location, when the nearest existing directory cannot be listed. The pane now moves up to a directory it can read, or to the home directory.
