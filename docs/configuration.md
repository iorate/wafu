# Configuration

wafu reads its settings from a TOML file:

| OS | Path |
|---|---|
| Windows | `%LOCALAPPDATA%\wafu\config.toml` |
| macOS | `~/Library/Application Support/wafu/config.toml` |
| Linux | `$XDG_CONFIG_HOME/wafu/config.toml`, or `~/.config/wafu/config.toml` |

Pass `--config PATH` to use another file; it is created the same way if missing.

wafu creates the file on first launch with a link to this page and nothing else. The `edit_config` command (`Z` in the file list) opens it in the [editor](#general).

- Changes apply as soon as the file is saved, except `use_login_shell_env`, which is read only when wafu starts.
- An item you leave out takes its default.
- An unknown item or an unusable value is reported in the log and ignored; the rest of the file still applies.
- A TOML syntax error is reported in the log with its line number, and the whole file is ignored: wafu keeps the configuration it had (the defaults, at startup).

## Defaults

```toml
[general]
mkdir_then_enter = true
new_text_creates_file = true
extract_creates_dir = true
require_marks = true
hide_log_in_preview = false
use_login_shell_env = true

[appearance]
theme = "auto"
size_format = "plain"
hidden_opacity = 0.7

[appearance.font]
family = ""
size = 12

[colors.light]
fg = "#1e1e1e"
bg = "#ffffff"
line-number = "#626262"
mark = "#dcdcdc"
cursor = "#1e1e1e"
cursor-inactive = "#7d7d7d"
dir = "#1f5fbf"
link = "#0e7c86"
exec = "#2a8c2a"
broken = "#c0392b"
fifo = "#b8860b"
socket = "#b03a9a"

[colors.dark]
fg = "#d4d4d4"
bg = "#1e1e1e"
line-number = "#9d9d9d"
mark = "#3c3c3c"
cursor = "#d4d4d4"
cursor-inactive = "#888888"
dir = "#6fa8ff"
link = "#4fd1e0"
exec = "#5fd75f"
broken = "#ff6b6b"
fifo = "#f0c050"
socket = "#e58ae0"

# The default editor depends on the OS.
[os.windows.general]
editor = "notepad {paths}"

[os.macos.general]
editor = "open -t {paths}"

[os.linux.general]
editor = "xdg-open {paths}"
```

There are no default bookmarks or external commands. Without a `[keys]` table, the keys shown in the [command palette](#keys) are the defaults.

If you copy this block, note that an item in an `[os.*]` section overrides the same item written at the top level (see [Per-OS sections](#per-os-sections)): to set one `editor` for every OS, write it under `[general]` and leave out the `[os.*.general]` tables.

## General

| Item | Meaning |
|---|---|
| `mkdir_then_enter` | Enter the directory that `mkdir` created. |
| `new_text_creates_file` | `new_text` creates the file before opening it in the editor. |
| `extract_creates_dir` | `extract` puts the contents in a directory named after the archive. |
| `require_marks` | `copy`, `move`, `delete`, and `trash` act only on marked items, never on the cursor item alone. |
| `editor` | The command line that opens a file for editing, used by `edit`, `new_text`, `preview_edit`, and `edit_config`. `{paths}` is the file. It follows the rules of [external commands](#commands). |
| `hide_log_in_preview` | Hide the log while a preview covers both panes. |
| `use_login_shell_env` | macOS and Linux: give the programs wafu starts the environment of your login shell. |

With `use_login_shell_env`, wafu runs your shell once at startup as an interactive login shell and takes its environment, so programs started from a desktop launcher still see what your shell startup files set up. While it does this, `WAFU_RESOLVING_ENVIRONMENT` is set to `1`, so startup files can skip things that should not run there:

```sh
[ -z "$WAFU_RESOLVING_ENVIRONMENT" ] && exec tmux
```

## Appearance

| Item | Values |
|---|---|
| `theme` | `"auto"` (follow the OS), `"light"`, or `"dark"` |
| `size_format` | `"plain"` (`1234567`), `"comma"` (`1,234,567`), or `"human"` (`1.2M`) |
| `hidden_opacity` | How opaque the names of hidden items are, from `0` (invisible) to `1` (like other items) |

### Fonts

`[appearance.font]` sets the font for the whole window:

- `family`: a font family name. Empty means the monospaced font of the OS (Consolas on Windows, Menlo on macOS, DejaVu Sans Mono on Linux).
- `size`: in pixels, from 6 to 72.

Each area can have its own font in a table with the same items: `[appearance.font.path]`, `[appearance.font.filelist]`, `[appearance.font.info]`, `[appearance.font.log]`, `[appearance.font.status]`, `[appearance.font.preview]`, and `[appearance.font.popup]`. An item an area leaves out (or an empty `family`) follows `[appearance.font]`.

```toml
[appearance.font.path]
family = "Consolas"
size = 16
```

## Colors

`[colors.light]` is used with the light theme and `[colors.dark]` with the dark one. Each color is written `"#rrggbb"` or `"#rgb"`.

| Item | Used for |
|---|---|
| `fg` | Text: regular files and the rest of the window's text |
| `bg` | Background |
| `line-number` | Line numbers in the text preview, and offsets in its binary mode |
| `mark` | Background of marked rows |
| `cursor` | The cursor line |
| `cursor-inactive` | The cursor line while the window is not focused |
| `dir` | Directories |
| `link` | Symbolic links whose target exists |
| `broken` | Symbolic links whose target does not exist |
| `exec` | Executable files |
| `fifo` | Named pipes and devices |
| `socket` | Sockets |

## Bookmarks

Each `[[bookmarks]]` entry adds one bookmark, in order. `name` is what the bookmarks popup shows; a `path` that is `~` or starts with `~/` (on Windows also `~\`) is taken from your home directory.

```toml
[[bookmarks]]
name = "Downloads"
path = "~/Downloads"

[[bookmarks]]
name = "Projects"
path = "~/projects"
```

## Commands

`[commands.NAME]` defines an external command with its command line in `run`. Bind it to a key in `[keys.filelist]` as `"$NAME"`.

```toml
[commands.open_terminal]
run = "wt -d {dir}"

[keys.filelist]
"F2" = "$open_terminal"
```

`wt` in the example is Windows Terminal.

The command runs without a shell, in the directory of the active pane. The line is split into arguments by the OS's own quoting rules:

- macOS and Linux: those of `sh`, with `'...'`, `"..."`, and `\`. Nothing is expanded: `$VAR`, `~`, and wildcards are passed as written, and a word starting with `#` begins a comment.
- Windows: those of the C runtime, with `"..."`. In the arguments, `\"` is a literal quote; single quotes are ordinary characters.

These placeholders are replaced when the command runs:

| Placeholder | Becomes |
|---|---|
| `{cursor_path}` | The item under the cursor (on the `..` row, the current directory) |
| `{marked_paths}` | The marked items, one argument each (none when nothing is marked) |
| `{paths}` | The marked items, or the item under the cursor when nothing is marked |
| `{dir}` | The directory of the active pane |
| `{other_dir}` | The directory of the other pane |

A placeholder must be a whole argument: `{dir}` is replaced, but `--dir={dir}` is passed as written, as is any other text in braces. Paths are full paths. Inside an archive, a line that uses `{cursor_path}`, `{marked_paths}`, or `{paths}` first extracts the items to a temporary directory, and those placeholders name the extracted files. `{dir}` is then the directory that holds the archive.

The same rules apply to `editor`.

## Keys

`[keys.filelist]`, `[keys.image]`, and `[keys.text]` bind keys in the file list, the image preview, and the text preview (its binary mode included). Each entry maps a key to a command name:

```toml
[keys.filelist]
"Q" = "nop"
"F4" = "edit"
```

- A key you bind replaces the built-in binding of that key only; all other built-in bindings stay.
- `"nop"` unbinds the key.
- `"$NAME"` runs `[commands.NAME]`.
- A command that does not work in the context is reported and ignored.

The command palette (`F1`) lists the commands available where you are, with the keys bound to them.

Keys are written as modifiers and a key joined by `+`, as in `"Ctrl+Shift+Left"`:

- Modifiers: `Ctrl`, `Shift`, and `Alt`. `Ctrl` is the Control key on every OS, including macOS.
- Letters: `A` to `Z`. A letter means the key without Shift (`"a"` and `"A"` are the same key); write `"Shift+A"` for the capital.
- Digits and symbols: the character as typed, such as `"1"`, `"@"`, `":"`, or `"+"`. Never add `Shift`: Shift+1 is written as the character it types, such as `"!"`.
- Named keys: `Enter`, `Space`, `Tab`, `BS`, `Esc`, `Up`, `Down`, `Left`, `Right`, `PageUp`, `PageDown`, `Home`, `End`, `Insert`, `Delete`, and `F1` to `F12`.

Modifier and key names are not case-sensitive.

## Per-OS sections

`[os.windows]`, `[os.macos]`, and `[os.linux]` hold items for one OS, written as in the rest of the file. On that OS they are merged over the top-level items; the other sections are ignored.

- Tables merge item by item: `[os.macos.general]` with only `editor` changes only `editor`.
- Key tables merge key by key.
- Arrays, such as `bookmarks`, replace the top-level array as a whole.

```toml
[[bookmarks]]
name = "Home"
path = "~"

[os.windows]
bookmarks = [
  { name = "Home", path = "~" },
  { name = "D", path = "D:\\" },
]

[os.macos.keys.filelist]
"Ctrl+Space" = "nop"
```
