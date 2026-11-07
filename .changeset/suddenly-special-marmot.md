---
wafu: patch
---

On Windows, `theme = "auto"` now follows a switch between light and dark mode made by PowerToys Light Switch and other tools that announce the change only once. The setting is read from the registry instead of the notification the window receives.
