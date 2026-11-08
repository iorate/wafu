---
wafu: minor
---

Keys during a preview come from their own tables. The text and binary previews' movement keys are now commands (preview_scroll_up, preview_page_down, preview_top, preview_toggle_mode, and so on), Enter and Esc close a preview through close_preview, and the log keys (Alt+Up/Down, Shift+Up/Down) work there too. The palette (F1) opens during a preview and lists the commands available in it. [keys.image], [keys.text], and [keys.binary] each have full default bindings and no longer inherit changes made under [keys.filelist].
