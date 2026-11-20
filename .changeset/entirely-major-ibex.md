---
wafu: patch
---

Treat names that differ only in Unicode normalization (NFC and NFD) as the same name: the cursor lands on an entry reached by a typed path, `rename` does nothing when the name is retyped, and a case-only rename of a decomposed name works.
