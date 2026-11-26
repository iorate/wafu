---
wafu: minor
---

An external command can now be written as a plain string, `NAME = "..."` under `[commands]`, as well as a table with `run`. An empty string in either form now takes back a command defined by a file below, instead of being reported as invalid.
