---
wafu: patch
---

A key in `keys` written as a space or a control character (such as `" "`, `"\t"` or `"Ctrl+ "`) is now reported as invalid, since it can never match a key press. Write `Space` or `Tab` instead.
