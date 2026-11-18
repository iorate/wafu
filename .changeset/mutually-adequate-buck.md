---
wafu: minor
---

Sizes outside the file list's size column are now always written exactly, with thousands separators, as in `1,234 bytes`; `size_format` applies only to that column. `calculate_size` and `calculate_size_on_disk` also log an approximation and list directories first, as in `Size of docs: 48,318,382,080 bytes (45GB) in 3 directories and 12 files`, and counts of one now read `1 item`, `1 byte`, and `1 match`.
