---
wafu: patch
---

The image preview now refuses an image that would need more than 512 MiB of memory to decode, instead of trying to allocate it. A small file that declares huge dimensions could otherwise freeze or crash wafu.
