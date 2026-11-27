# wafu

A keyboard-driven dual-pane file manager for Windows, macOS, and Linux, built with Rust and Slint.

![wafu with a project directory in the left pane, three files marked and the cursor on README.md, a downloads directory in the right pane, and the log below them](https://raw.githubusercontent.com/iorate/wafu/main/docs/images/main.png)

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)

## Features

The default keys are modeled on [あふｗ](https://afxw.sakura.ne.jp/), a two-pane file manager for Windows; wafu is not compatible with it and does not aim to be.

- **Keyboard first.** Most commands are one key: <kbd>Space</kbd> marks, <kbd>R</kbd> renames, <kbd>D</kbd> deletes, <kbd>K</kbd> makes a directory. <kbd>F1</kbd> opens a command palette that lists the commands available where you are, with their keys, and narrows the list as you type. Key bindings can be changed in the configuration file.
- **Two panes.** <kbd>C</kbd> copies and <kbd>M</kbd> moves the marked items to the other pane. <kbd>W</kbd> compares the panes and marks what the other side is missing, or also what is newer or different there.
- **Archives as directories.** <kbd>Enter</kbd> opens a zip, tar, tar.gz, tar.bz2, or tar.xz archive like a directory, and an archive inside it the same way. <kbd>U</kbd> extracts and <kbd>P</kbd> compresses, zip optionally with a password. Archives are read-only: you can copy and extract from one, but not add, change, or delete anything inside it.
- **Batch rename with a live preview.** <kbd>Shift</kbd>+<kbd>R</kbd> finds and replaces text in the names of the marked files, and <kbd>Alt</kbd>+<kbd>R</kbd> makes each name from a template with `{name}`, `{ext}`, and a counter such as `{001}`. Each name that changes is listed with its new name as you type, and a clash such as a duplicate name is shown before anything is renamed.

  ![The template rename popup with the template kyoto_{001}{ext}, listing each marked photo's old name beside its new one, starting with IMG_0408.jpg and kyoto_001.jpg](https://raw.githubusercontent.com/iorate/wafu/main/docs/images/rename.png)

- **Previews.** <kbd>Enter</kbd> on an image in the left pane shows it in the right half, and the preview follows the cursor. Other files open as text (<kbd>V</kbd> forces it); there <kbd>Tab</kbd> switches to a hex dump and <kbd>C</kbd> picks the encoding.

  ![A photo of hills at sunset previewed in the right half of the window, beside the list of photos it was opened from, one of which is marked](https://raw.githubusercontent.com/iorate/wafu/main/docs/images/preview.png)

- **Getting around.** <kbd>F</kbd> searches the list as you type, <kbd>:</kbd> filters it with a glob such as `*.{jpg,png}`, and <kbd>Shift</kbd>+<kbd>F</kbd> finds files by name in all subdirectories. Each pane keeps its own history (<kbd>H</kbd>), and <kbd>J</kbd> opens your bookmarks.
- **Configuration applied on save.** <kbd>Z</kbd> opens the configuration file in your editor, and changes apply as soon as it is saved.

## Installation

### Windows

Install with [Scoop](https://scoop.sh):

```sh
scoop bucket add iorate https://github.com/iorate/scoop-bucket
scoop install iorate/wafu
```

### macOS

Install with [Homebrew](https://brew.sh) (Apple silicon only):

```sh
brew install iorate/tap/wafu
```

This installs the `wafu` command and `wafu.app`. `brew info iorate/tap/wafu` shows where the app is and how to put it in `Applications`.

### Linux

Install with [Homebrew](https://brew.sh) (x86-64 only):

```sh
brew install iorate/tap/wafu
```

The binary needs glibc 2.28 or later and uses the libraries of the host: fontconfig, xkbcommon, Wayland or X11, and EGL or OpenGL.

The directory dialog of `browse` needs an XDG Desktop Portal backend with a file chooser (GTK, GNOME, or KDE) or, failing that, `zenity`.

### Cargo

On any of the three, wafu can be built from source instead:

```sh
cargo install --locked wafu
```

On Linux, install the build dependencies first. On Debian and Ubuntu:

```sh
sudo apt install libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libxkbcommon-x11-dev libfontconfig-dev libwayland-dev
```

## Configuration

Settings, colors, bookmarks, external commands, and key bindings live in a TOML file. See [docs/configuration.md](https://github.com/iorate/wafu/blob/main/docs/configuration.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
