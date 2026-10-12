# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project Overview

`wafu` is a keyboard-driven dual-pane file manager for Windows, Linux, and macOS, built with Rust and Slint. Cargo workspace:

- `wafu-core`: the pure model. `App::update(Msg) -> Vec<Effect>` and `App::view() -> View`. No I/O, no Slint.
- `wafu-io`: executes `Effect`s (file system, archives, watching, OS integration) and reports back as `IoEvent`s.
- `wafu`: the binary. Slint UI, event loop, and wiring between the two.

## Verifying Changes

After editing, run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
