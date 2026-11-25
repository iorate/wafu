---
wafu: patch
---

A pane's location is now normalized however it is reached, so going up from a location given with `..` works, Windows locations always use `\`, and `D:` goes to `D:\`. A relative bookmark path is now taken from the directory of the configuration file it is written in.
