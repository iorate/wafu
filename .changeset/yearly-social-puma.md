---
wafu: patch
---

Opening a file that is not a zip now says "not a zip archive" (or "not an archive" when the name gives no format), a broken zip says "the zip archive is damaged", and the entry is named when the damage is in one, instead of the zip library's own message; for an encrypted entry, the message also says the password may be wrong.
