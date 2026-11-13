# Configuration

wafu reads its settings from a TOML file:

| OS | Path |
|---|---|
| Windows | `%LOCALAPPDATA%\wafu\config.toml` |
| macOS | `~/Library/Application Support/wafu/config.toml` |
| Linux | `$XDG_CONFIG_HOME/wafu/config.toml`, or `~/.config/wafu/config.toml` |

Pass `--config PATH` to use another file; it is created the same way if missing.

wafu creates the file on first launch with a link to this page and nothing else. The `edit_config` command (`Z` in the file list) opens it in the [editor](#programs).

- Changes apply as soon as the file is saved, except `use_login_shell_env`, which is read only when wafu starts.
- An item you leave out takes its default.
- An unknown item or an unusable value is reported in the log and ignored; the rest of the file still applies.
- A TOML syntax error is reported in the log with its line number, and the whole file is ignored: wafu keeps the configuration it had (the defaults, at startup).

## Defaults

```toml
[filelist]
mkdir_enters_dir = true
new_text_creates_file = true
extract_creates_dir = true
require_marks = true
search_matches_anywhere = true
size_format = "plain"

[preview.text]
hide_log = false

[preview.image]
from_left = "other_pane"
from_right = "both_panes"
hide_log = false

[programs]
# editor = "notepad {paths}" on Windows, "open -t {paths}" on macOS, "xdg-open {paths}" on Linux
use_login_shell_env = true

[font]
family = ""
size = 12

[colors]
theme = "auto"
hidden_opacity = 0.7

[colors.light]
fg = "#1e1e1e"
bg = "#ffffff"
line_number = "#626262"
mark = "#dcdcdc"
cursor = "#1e1e1e"
cursor_inactive = "#7d7d7d"
dir = "#1f5fbf"
link = "#0e7c86"
exec = "#2a8c2a"
broken = "#c0392b"
fifo = "#b8860b"
socket = "#b03a9a"

[colors.dark]
fg = "#d4d4d4"
bg = "#1e1e1e"
line_number = "#9d9d9d"
mark = "#3c3c3c"
cursor = "#d4d4d4"
cursor_inactive = "#888888"
dir = "#6fa8ff"
link = "#4fd1e0"
exec = "#5fd75f"
broken = "#ff6b6b"
fifo = "#f0c050"
socket = "#e58ae0"
```

There are no default bookmarks or external commands. Without a `[keys]` table, the keys shown in the [command palette](#keys) are the defaults.

## File list

| Item | Meaning |
|---|---|
| `mkdir_enters_dir` | Enter the directory that `mkdir` created. |
| `new_text_creates_file` | `new_text` creates the file before opening it in the editor. |
| `extract_creates_dir` | `extract` puts the contents in a directory named after the archive. |
| `require_marks` | `copy`, `move`, `delete`, and `trash` act only on marked items, never on the cursor item alone. |
| `search_matches_anywhere` | `search` finds the text anywhere in a name. When `false`, only at the start of a name. |
| `size_format` | How sizes are written: `"plain"` (`1234567`), `"comma"` (`1,234,567`), or `"human"` (`1.2M`). |

## Preview

`[preview.text]` is for the text preview, its binary mode included, which always takes the place of both panes:

| Item | Meaning |
|---|---|
| `hide_log` | Hide the log while a text preview is open. |

`[preview.image]` is for the image preview:

| Item | Meaning |
|---|---|
| `from_left` | Where an image opened from the left pane is shown: `"other_pane"` (in place of the right pane, while the left pane stays) or `"both_panes"`. |
| `from_right` | Where an image opened from the right pane is shown: `"other_pane"` (in place of the left pane, while the right pane stays) or `"both_panes"`. |
| `hide_log` | Hide the log while an image takes the place of both panes. |

## Programs

| Item | Meaning |
|---|---|
| `editor` | The command line that opens a file for editing, used by `edit`, `new_text`, `preview_edit`, and `edit_config`. `{paths}` is the file. It follows the rules of [external commands](#commands). |
| `use_login_shell_env` | macOS and Linux: give the programs wafu starts the environment of your login shell. |

With `use_login_shell_env`, wafu runs your shell once at startup as an interactive login shell and takes its environment, so programs started from a desktop launcher still see what your shell startup files set up. While it does this, `WAFU_RESOLVING_ENVIRONMENT` is set to `1`, so startup files can skip things that should not run there:

```sh
[ -z "$WAFU_RESOLVING_ENVIRONMENT" ] && exec tmux
```

## Fonts

`[font]` sets the font for the whole window:

- `family`: a font family name. Empty means the monospaced font of the OS (Consolas on Windows, Menlo on macOS, DejaVu Sans Mono on Linux).
- `size`: in pixels, from 6 to 72.

Each area can have its own font in a table with the same items: `[font.path]`, `[font.filelist]`, `[font.info]`, `[font.log]`, `[font.status]`, `[font.preview]`, and `[font.popup]`. An item an area leaves out (or an empty `family`) follows `[font]`.

```toml
[font.path]
family = "Consolas"
size = 16
```

## Colors

| Item | Values |
|---|---|
| `theme` | `"auto"` (follow the OS), `"light"`, or `"dark"` |
| `hidden_opacity` | How opaque the names of hidden items are, from `0` (invisible) to `1` (like other items) |

`[colors.light]` is used with the light theme and `[colors.dark]` with the dark one. Each color is written `"#rrggbb"` or `"#rgb"`.

| Item | Used for |
|---|---|
| `fg` | Text: regular files and the rest of the window's text |
| `bg` | Background |
| `line_number` | Line numbers in the text preview, and offsets in its binary mode |
| `mark` | Background of marked rows |
| `cursor` | The cursor line |
| `cursor_inactive` | The cursor line while the window is not focused |
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
key = "D"

[[bookmarks]]
name = "Projects"
path = "~/projects"
```

`key` is optional: a digit or a letter that picks the bookmark at once in the popup.

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
