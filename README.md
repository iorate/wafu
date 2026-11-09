# wafu

A keyboard-driven dual-pane file manager for Windows, macOS, and Linux, built with Rust and Slint.

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)

## Installation

### Windows

Download `wafu-vX.Y.Z-windows-x64.zip` from [GitHub Releases](https://github.com/iorate/wafu/releases), extract it, and run `wafu.exe`.

The executable is not signed, so Microsoft Defender SmartScreen may block it on the first run. Click "More info" and then "Run anyway".

### macOS

Install with Cargo:

```sh
cargo install --locked wafu
```

Alternatively, download `wafu-vX.Y.Z-macos-arm64.dmg` (Apple silicon) from [GitHub Releases](https://github.com/iorate/wafu/releases) and drag `wafu.app` into `Applications`.

The app is not notarized, so macOS blocks it on the first launch. Open System Settings → Privacy & Security and click "Open Anyway", or remove the quarantine attribute:

```sh
xattr -dr com.apple.quarantine /Applications/wafu.app
```

When wafu is not started from a terminal (for example from the Finder or the Dock), it gets the environment for the programs it starts, such as the editor and external commands, from your login shell: it runs `$SHELL` as an interactive login shell in the background. Your shell's startup files can check the `WAFU_RESOLVING_ENVIRONMENT` variable to skip lines that should not run then, such as `exec tmux`:

```sh
[ -z "$WAFU_RESOLVING_ENVIRONMENT" ] && exec tmux
```

### Linux

Install the build dependencies and then wafu with Cargo. On Debian and Ubuntu:

```sh
sudo apt install libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libxkbcommon-x11-dev libfontconfig-dev libwayland-dev
cargo install --locked wafu
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
