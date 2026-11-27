<!-- What stands between a pair of "generated" comments is generated from the settings of wafu-core. Do not edit it by hand; regenerate with: WAFU_UPDATE_DOCS=1 cargo test -p wafu-core --test configuration_doc -->

# Configuration

wafu reads its settings from a TOML file:

| OS | Path |
|---|---|
| Windows | `%LOCALAPPDATA%\wafu\config.toml` |
| macOS | `~/Library/Application Support/wafu/config.toml` |
| Linux | `$XDG_CONFIG_HOME/wafu/config.toml`, or `~/.config/wafu/config.toml` |

Pass `--config PATH` to use another file; it is created the same way if missing.

wafu creates the file on first launch with a link to this page and nothing else. The `edit_config` command (`Z` in the file list) opens it in the [editor](#programs).

- Changes apply as soon as the file, or any file it [extends](#sharing-settings-between-machines), is saved, except `use_login_shell_env`, which is read only when wafu starts.
- An item you leave out takes its default.
- An unknown item or an unusable value is reported in the log and ignored; the rest of the configuration still applies.
- A TOML syntax error is reported in the log with the file and line number, and the whole configuration is ignored: wafu keeps the configuration it had (the defaults, at startup).

## Sharing settings between machines

To keep shared settings in one file, such as one in your dotfiles, and only what differs on each machine in the configuration file, name the shared file in `extends` at the top of the configuration file:

```toml
extends = "~/dotfiles/wafu/common.toml"      # or an array: extends = ["a.toml", "b.toml"]

[programs]
editor = "code {path}"
```

- The configuration file applies over what it extends, and each file in an array applies over the ones before it.
- Tables merge item by item, and the key tables (`[keys.filelist]` and the like) key by key. An array, such as `bookmarks`, replaces the one below it as a whole.
- For an item written as a string, such as a color or a command line, and for a value in an extension or key table, an empty string takes back what a file below set, as if no file had written the item: `pdf = ""` under `[open_alternative.extensions]` leaves `pdf` without a rule, and `editor = ""` brings back the default editor. An item that is `true` or `false` or a number is not taken back this way; write the value you want instead.
- A path that is `~` or starts with `~/` (on Windows also `~\`) is taken from your home directory; any other relative path is taken from the directory of the file that names it. If that file is a symbolic link, this is the directory the link is in, not the one it points to.
- A file you extend may have an `extends` of its own. A file that ends up extending itself is an error.
- If a file you extend is missing, cannot be read, or has a syntax error, the error is reported in the log with that file's path, and the whole configuration is ignored, as for a syntax error in the configuration file. Only the configuration file itself is ever created.
- wafu watches every file involved, so saving any of them applies the change.

For example, to use other bookmarks on one machine, write them all in its configuration file; they replace the shared bookmarks rather than adding to them:

```toml
extends = "~/dotfiles/wafu/common.toml"

[[bookmarks]]
name = "Work"
path = "~/work"
```

## Defaults

<!-- generated:defaults -->

```toml
[filelist]
new_dir_goes_to_dir = true
new_text_creates_file = true
extract_creates_dir = true
require_marks = true
search_match = "anywhere"
size_format = "plain"
size_digits = 9
time_shows_full_year = true
time_shows_seconds = true
extension_chars = 4
min_name_chars = 8
max_history = 100
max_find_matches = 10000
mark_compare_tolerance_ms = 2000

[preview.text]
hide_log = false
max_bytes = 16777216

[preview.image]
placement_from_left = "other_pane"
placement_from_right = "both_panes"
hide_log = false
max_bytes = 67108864

[log]
max_lines = 1000

[programs]
# editor = "notepad {path}" on Windows, "open -t {path}" on macOS, "xdg-open {path}" on Linux
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
link_broken = "#c0392b"
exec = "#2a8c2a"
special = "#b8860b"
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
link_broken = "#ff6b6b"
exec = "#5fd75f"
special = "#f0c050"
socket = "#e58ae0"
```

<!-- /generated -->

There are no default bookmarks, external commands, [`open_alternative` programs](#alternative-open), or [extension colors](#colors). Without a `[keys]` table, the keys shown in the [command palette](#keys) are the defaults.

## File list

<!-- generated:filelist -->

| Item | Value | Meaning |
|---|---|---|
| `new_dir_goes_to_dir` | true or false | Go to the directory that `new_dir` created. |
| `new_text_creates_file` | true or false | `new_text` creates the file before opening it in the editor. |
| `extract_creates_dir` | true or false | `extract` and `extract_here` put the contents in a directory named after the archive. |
| `require_marks` | true or false | `copy`, `move`, `delete`, and `trash` act only on marked items, never on the cursor item alone. |
| `search_match` | anywhere or start | Where in a name `search` finds the text: `"anywhere"` or `"start"` (only at the start of a name). |
| `size_format` | plain, comma, or human | How the file list's size column writes sizes: `"plain"` (`1234567`), `"comma"` (`1,234,567`), or `"human"` (`1.2M`). |
| `size_digits` | an integer from 5 to 20 | How many digits the size column holds with `"plain"` and `"comma"`. A size with more digits is written as with `"human"`. With `"human"`, the column always holds five characters. |
| `time_shows_full_year` | true or false | The file list's time column writes the year in full (`2026/01/02`). When `false`, only its last two digits (`26/01/02`). |
| `time_shows_seconds` | true or false | The file list's time column writes the seconds (`03:04:05`). When `false`, only the hours and minutes (`03:04`). |
| `extension_chars` | an integer from 1 to 255 | How many characters of an extension the extension column holds after the dot. A longer extension ends in an ellipsis. |
| `min_name_chars` | an integer from 1 to 255 | How many characters of a name a narrow pane still shows before cutting off the other columns. |
| `max_history` | an integer of 1 or more | How many places each pane's history keeps, for `history`, `go_back`, and `go_forward`. |
| `max_find_matches` | an integer of 1 or more | How many matches `find` collects before it stops. |
| `mark_compare_tolerance_ms` | an integer of 0 or more | How many milliseconds apart two modification times may be for `mark_compare` to take them as the same. `0` compares them exactly. |

<!-- /generated -->

## Preview

`[preview.text]` is for the text preview, its binary mode included, which always takes the place of both panes:

<!-- generated:preview.text -->

| Item | Value | Meaning |
|---|---|---|
| `hide_log` | true or false | Hide the log while a text preview is open. |
| `max_bytes` | an integer of 1 or more | How many bytes of a file the text preview reads. A longer file is cut there. |

<!-- /generated -->

`[preview.image]` is for the image preview:

<!-- generated:preview.image -->

| Item | Value | Meaning |
|---|---|---|
| `placement_from_left` | other_pane or both_panes | Where an image opened from the left pane is shown: `"other_pane"` (in place of the right pane, while the left pane stays) or `"both_panes"`. |
| `placement_from_right` | other_pane or both_panes | Where an image opened from the right pane is shown: `"other_pane"` (in place of the left pane, while the right pane stays) or `"both_panes"`. |
| `hide_log` | true or false | Hide the log while an image takes the place of both panes. |
| `max_bytes` | an integer of 1 or more | The largest image file, in bytes, the image preview opens. |

<!-- /generated -->

## Log

<!-- generated:log -->

| Item | Value | Meaning |
|---|---|---|
| `max_lines` | an integer of 1 or more | How many lines the log keeps. |

<!-- /generated -->

## Programs

<!-- generated:programs -->

| Item | Value | Meaning |
|---|---|---|
| `editor` | a string | The command line that opens a file for editing, used by `edit`, `new_text`, `preview_edit`, and `edit_config`. It follows the rules of [external commands](#commands), with `{path}`, the file, as its only placeholder. |
| `use_login_shell_env` | true or false | macOS and Linux: give the programs wafu starts the environment of your login shell. |

<!-- /generated -->

With `use_login_shell_env`, wafu runs your shell once at startup as an interactive login shell and takes its environment, so programs started from a desktop launcher still see what your shell startup files set up. While it does this, `WAFU_RESOLVING_ENVIRONMENT` is set to `1`, so startup files can skip things that should not run there:

```sh
[ -z "$WAFU_RESOLVING_ENVIRONMENT" ] && exec tmux
```

## Fonts

`[font]` sets the font for the whole window:

<!-- generated:font -->

| Item | Value | Meaning |
|---|---|---|
| `family` | a string | A font family name. Empty means the monospaced font of the OS (Consolas on Windows, Menlo on macOS, DejaVu Sans Mono on Linux). |
| `size` | an integer from 6 to 72 | The font size, in pixels. |

<!-- /generated -->

Each area can have its own font in a table with the same items: `[font.path]`, `[font.filelist]`, `[font.info]`, `[font.log]`, `[font.status]`, `[font.preview]`, and `[font.popup]`. An item an area leaves out (or an empty `family`) follows `[font]`.

```toml
[font.path]
family = "Consolas"
size = 16
```

## Colors

<!-- generated:colors -->

| Item | Value | Meaning |
|---|---|---|
| `theme` | auto, light, or dark | Which colors are used: `"auto"` (follow the OS), `"light"`, or `"dark"`. |
| `hidden_opacity` | a number from 0 to 1 | How opaque the names of hidden items are: `0` is invisible, and `1` is like other items. |

<!-- /generated -->

`[colors.light]` is used with the light theme and `[colors.dark]` with the dark one. Each color is written `"#rrggbb"` or `"#rgb"`.

<!-- generated:colors.themes -->

| Item | Used for |
|---|---|
| `fg` | Text: regular files and the rest of the window's text |
| `bg` | Background |
| `line_number` | Line numbers in the text preview, and offsets in its binary mode |
| `mark` | Background of marked rows and of the preview of a marked image |
| `cursor` | The cursor line |
| `cursor_inactive` | The cursor line while the window is not focused |
| `dir` | Directories |
| `link` | Symbolic links whose target exists |
| `link_broken` | Symbolic links whose target does not exist |
| `exec` | Executable files |
| `special` | Named pipes and devices |
| `socket` | Sockets |

<!-- /generated -->

`[colors.light.extensions]` and `[colors.dark.extensions]` give regular files a color by extension, in place of `fg`. Each key is an extension without the dot, such as `zip` for `a.zip`, and matches in either case, so `zip` also colors `B.ZIP`. Only the part after the last dot is matched: `gz` colors `a.tar.gz`, and a key with a dot, such as `"tar.gz"`, is reported as invalid. The other kinds above keep their own color, so an executable `run.sh` takes `exec` whatever `sh` is set to. On Windows, `exe`, `com`, `bat`, and `cmd` files are executable, so they take `exec` too. Each theme has its own table: an extension set in only one of them takes `fg` in the other. Both tables are empty by default.

```toml
[colors.light.extensions]
zip = "#a02020"
7z = "#a02020"

[colors.dark.extensions]
zip = "#ff8080"
7z = "#ff8080"
```

## Bookmarks

Each `[[bookmarks]]` entry adds one bookmark, in order. `name` is what the bookmarks popup shows; a `path` that is `~` or starts with `~/` (on Windows also `~\`) is taken from your home directory, and any other relative path from the directory of the file it is written in.

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

`NAME = "..."` under `[commands]` defines an external command with its command line. Bind it to a key in `[keys.filelist]` as `"$NAME"`.

```toml
[commands]
open_terminal = "wt -d {dir}"

[keys.filelist]
"F2" = "$open_terminal"
```

`wt` in the example is Windows Terminal.

A command can also be written as a table with the line in `command_line`: `open_terminal = { command_line = "wt -d {dir}" }`, or `[commands.open_terminal]` with `command_line = "..."` under it. An empty string in either form leaves the command undefined, which is how a file takes back a command that a file it [extends](#sharing-settings-between-machines) defined.

The command runs without a shell, in the directory of the active pane. The line is split into arguments by the OS's own quoting rules:

- macOS and Linux: those of `sh`, with `'...'`, `"..."`, and `\`. Nothing is expanded: `$VAR`, `~`, and wildcards are passed as written, and a word starting with `#` begins a comment.
- Windows: those of the C runtime, with `"..."`. In the arguments, `\"` is a literal quote; single quotes are ordinary characters.

These placeholders are replaced when the command runs:

| Placeholder | Becomes |
|---|---|
| `{cursor_path}` | The item under the cursor (on the `..` row, the current directory) |
| `{marked_paths}` | The marked items |
| `{paths}` | The marked items, or the item under the cursor when nothing is marked |
| `{dir}` | The directory of the active pane |
| `{other_dir}` | The directory of the other pane |

A placeholder is replaced wherever it stands in an argument: `-o{other_dir}` and `--dir={dir}` work. Quoting does not turn it into text. `{marked_paths}` and `{paths}` are lists: an argument that holds one is repeated for each path, so `{paths}` becomes one argument per item and `--add={marked_paths}` becomes `--add=/a --add=/b`. An argument can hold only one list. The program, the first word of the line, cannot hold a placeholder.

To write a brace as text, double it: `{{` and `}}`. Any other brace is an error, as is a name in braces that is not in the table.

When a placeholder in the line has no value, such as `{marked_paths}` with nothing marked, the command is not started and the log says which placeholder was empty.

Paths are full paths. Inside an archive, a line that uses `{cursor_path}`, `{marked_paths}`, or `{paths}` first extracts the items to a temporary directory, and those placeholders name the extracted files. There, on a `..` row other than the one at the root of the outermost archive, `{cursor_path}` is empty while items are marked. `{dir}` is then the directory that holds the archive file, the outermost one for an archive inside an archive.

The same rules apply to the lines typed into the dialogs of `run` and `run_command_line`. `run` doubles the braces in the path it starts out with.

`editor` and the lines of [alternative open](#alternative-open) follow these rules too, but run on one file rather than on the pane, so their only placeholder is `{path}`, that file. The placeholders in the table are errors there.

## Alternative open

`open_alternative` (`Shift+Enter` in the file list) opens the item under the cursor in a program you choose for its kind, in place of what `open` (`Enter`) does:

```toml
[open_alternative]
dir = "code {path}"

[open_alternative.extensions]
pdf = "sumatrapdf {path}"
```

- `dir` is the command line for directories, and links to them.
- `[open_alternative.extensions]` gives regular files, and links to them, a command line by extension. Its keys follow the rules of [extension colors](#colors): an extension without the dot, matched in either case and against the part after the last dot only. A rule for an extension wins over opening an archive as a directory or running an executable file, so a `zip` rule opens `a.zip` in its program rather than going into it.
- On the `..` row the item is the current directory itself, or, at the root of an archive, the archive file.
- Anything without a rule, broken links, special files, and links inside an archive are opened as `open` does.
- Both are empty by default.

The command lines follow the rules of [external commands](#commands), with `{path}`, the item under the cursor, as their only placeholder. Inside an archive, a line that uses `{path}` first extracts the item to a temporary directory, as for external commands.

## Keys

`[keys.filelist]`, `[keys.preview.image]`, and `[keys.preview.text]` bind keys in the file list, the image preview, and the text preview (its binary mode included). Each entry maps a key to a command name:

```toml
[keys.filelist]
"Q" = "nop"
"F4" = "edit"
```

- A key you bind replaces the built-in binding of that key only; all other built-in bindings stay.
- `"nop"` unbinds the key.
- `""` gives the key back its built-in binding.
- `"$NAME"` runs the command `NAME` of `[commands]`.
- A command that does not work in the context is reported and ignored.

The command palette (`?`) lists the commands available where you are, with the keys bound to them. [commands.md](commands.md) lists every command with its default keys and what it does.

Keys are written as modifiers and a key joined by `+`, as in `"Ctrl+Shift+Left"`:

- Modifiers: `Ctrl`, `Alt`, and `Shift`, in any order. `Ctrl` is the Control key on every OS, including macOS.
- Letters: `A` to `Z`. A letter means the key without Shift (`"a"` and `"A"` are the same key); write `"Shift+A"` for the capital.
- Digits and symbols: the character as typed, such as `"1"`, `"@"`, `":"`, or `"+"`. Never add `Shift`: Shift+1 is written as the character it types, such as `"!"`.
- Named keys: `Enter`, `Space`, `Tab`, `Backspace`, `Esc`, `Up`, `Down`, `Left`, `Right`, `PageUp`, `PageDown`, `Home`, `End`, `Insert`, `Delete`, and `F1` to `F12`.

Modifier and key names are not case-sensitive.
