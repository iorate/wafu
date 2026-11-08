---
wafu: minor
---

When an entry cannot be extracted (an unsafe or invalid name, a link that cannot be made, an unsupported entry), extraction and copies out of an archive now log the entry, skip it, and go on; the summary line shows how many entries were skipped. Before, the whole operation stopped at the first such entry. Edit and open from inside an archive do not start the program when an entry was skipped.
