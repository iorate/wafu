---
wafu: patch
---

The image preview now refuses an image that would need more than 1 GiB of memory to decode, instead of trying to allocate it. A small file that declares huge dimensions could otherwise freeze or crash wafu.
