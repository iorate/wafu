# wafu

A keyboard-driven dual-pane file manager for Windows, macOS, and Linux, built with Rust and Slint.

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)

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
