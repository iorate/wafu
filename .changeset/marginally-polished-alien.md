---
wafu: patch
---

Extracting a zip that starts with `./` no longer reports skipped entries, and when a zip holds the same file under two spellings, such as `a.txt` and `./a.txt`, only the later one is listed and extracted.
