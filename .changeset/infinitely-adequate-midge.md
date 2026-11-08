---
wafu: patch
---

A zip entry in a compression method wafu cannot read, such as LZMA, is now skipped and logged while the rest of the archive is extracted, instead of stopping the extraction.
