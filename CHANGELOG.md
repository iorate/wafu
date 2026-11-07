# wafu

## 0.2.0

### Minor Changes

- The cursor line takes a new color, `cursor-inactive`, while the window is not focused, so that it shows whether keys reach wafu. The default is a grey between the cursor and the mark background.

### Patch Changes

- On Windows, `theme = "auto"` now follows a switch between light and dark mode made by PowerToys Light Switch and other tools that announce the change only once. The setting is read from the registry instead of the notification the window receives.

- A double click on the left pane while an image is previewed in the right pane now closes the preview, as a double click on the image already does. The cursor moves to the clicked row first.

## 0.1.0

- Initial release.
