---
wafu: patch
---

An empty `extends` (`extends = ""`) now extends nothing, like an empty string for other string items, instead of making wafu ignore the whole configuration. An empty string inside an `extends` array is reported as invalid and skipped.
