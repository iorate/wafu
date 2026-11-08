---
wafu: patch
---

Zip archives that wafu creates now record each modification time to the second in UTC, so odd seconds, times before 1980, and, for wafu and other tools that read extended timestamps, opening the archive in another time zone no longer shift the times.
