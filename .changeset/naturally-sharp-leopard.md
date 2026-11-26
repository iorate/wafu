---
wafu: patch
---

Fix extraction stopping at a zip symbolic link whose target is not UTF-8, and skip a link whose target is longer than 1 MiB, without reading it into memory when it is in a zip. On Windows, extraction no longer connects to a network host named by a link target; a link is made as a directory link only when its target is a relative path to a directory with no other link on the way.
