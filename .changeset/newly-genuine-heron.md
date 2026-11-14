---
wafu: minor
---

An empty string in the configuration now means the item is not set, so a configuration file can take back what a file it extends set, such as `pdf = ""` for an `open_alternative` rule. In a key table, `""` restores the built-in binding.
