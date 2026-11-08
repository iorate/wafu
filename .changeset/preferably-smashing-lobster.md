---
wafu: patch
---

Zip archives that wafu creates now record each modification time to the second in UTC, so odd seconds, times before 1980, and opening the archive in another time zone no longer shift the times.
