<!-- What stands between a pair of "generated" comments is generated from the settings of wafu-core. Do not edit it by hand; regenerate with: WAFU_UPDATE_DOCS=1 cargo test -p wafu-core --test configuration_doc -->

# Configuration

wafu reads its settings from a TOML file and applies a change as soon as the file is saved.

| OS | Path |
|---|---|
| Windows | `%LOCALAPPDATA%\wafu\config.toml` |
| macOS | `~/Library/Application Support/wafu/config.toml` |
| Linux | `$XDG_CONFIG_HOME/wafu/config.toml`, or `~/.config/wafu/config.toml` |

`--config PATH` uses another file. The `edit_config` command (`Z` in the file list) opens the file in the [editor](#programs).

An item you leave out takes its default. A mistake in the file is reported in the log.

## Sharing settings between machines

`extends` names a file, such as one in your dotfiles, that the configuration file builds on:

```toml
extends = "~/dotfiles/wafu/common.toml"      # or an array: extends = ["a.toml", "b.toml"]

[programs]
editor = "code {path}"
```

The configuration file applies over what it extends, and each file in an array over the ones before it. Tables merge item by item; an array, such as `bookmarks`, is replaced as a whole. An empty string takes back what an extended file set: `editor = ""` brings back the default editor.

A relative path is taken from the directory of the file that names it.

## File list

<!-- generated:filelist -->

| Item | Type | Default | Meaning |
|---|---|---|---|
| `new_dir_goes_to_dir` | true or false | `true` | Go to the directory that `new_dir` created. |
| `new_text_creates_file` | true or false | `true` | `new_text` creates the file before opening it in the editor. |
| `extract_creates_dir` | true or false | `true` | `extract` and `extract_here` put the contents in a directory named after the archive. |
| `require_marks` | true or false | `true` | `copy`, `move`, `delete`, and `trash` act only on marked items, never on the cursor item alone. |
| `search_match` | anywhere or start | `"anywhere"` | Where in a name `search` finds the text: `"anywhere"` or `"start"` (only at the start of a name). |
| `size_format` | plain, comma, or human | `"plain"` | How the file list's size column writes sizes: `"plain"` (`1234567`), `"comma"` (`1,234,567`), or `"human"` (`1.2M`). |
| `size_digits` | an integer from 5 to 20 | `9` | How many digits the size column holds with `"plain"` and `"comma"`. A size with more digits is written as with `"human"`. With `"human"`, the column always holds five characters. |
| `time_shows_full_year` | true or false | `true` | The file list's time column writes the year in full (`2026/01/02`). When `false`, only its last two digits (`26/01/02`). |
| `time_shows_seconds` | true or false | `true` | The file list's time column writes the seconds (`03:04:05`). When `false`, only the hours and minutes (`03:04`). |
| `extension_chars` | an integer from 1 to 255 | `4` | How many characters of an extension the extension column holds after the dot. A longer extension ends in an ellipsis. |
| `min_name_chars` | an integer from 1 to 255 | `8` | How many characters of a name a narrow pane still shows before cutting off the other columns. |
| `max_history` | an integer of 1 or more | `100` | How many places each pane's history keeps, for `history`, `go_back`, and `go_forward`. |
| `max_find_matches` | an integer of 1 or more | `10000` | How many matches `find` collects before it stops. |
| `mark_compare_tolerance_ms` | an integer of 0 or more | `2000` | How many milliseconds apart two modification times may be for `mark_compare` to take them as the same. `0` compares them exactly. |

<!-- /generated -->

## Preview

`[preview.text]` is for the text preview, its binary mode included:

<!-- generated:preview.text -->

| Item | Type | Default | Meaning |
|---|---|---|---|
| `hide_log` | true or false | `false` | Hide the log while a text preview is open. |
| `max_bytes` | an integer of 1 or more | `16777216` | How many bytes of a file the text preview reads. A longer file is cut there. |

<!-- /generated -->

`[preview.image]` is for the image preview:

<!-- generated:preview.image -->

| Item | Type | Default | Meaning |
|---|---|---|---|
| `placement_from_left` | other_pane or both_panes | `"other_pane"` | Where an image opened from the left pane is shown: `"other_pane"` (in place of the right pane, while the left pane stays) or `"both_panes"`. |
| `placement_from_right` | other_pane or both_panes | `"both_panes"` | Where an image opened from the right pane is shown: `"other_pane"` (in place of the left pane, while the right pane stays) or `"both_panes"`. |
| `hide_log` | true or false | `false` | Hide the log while an image takes the place of both panes. |
| `max_bytes` | an integer of 1 or more | `67108864` | The largest image file, in bytes, the image preview opens. |

<!-- /generated -->

## Log

<!-- generated:log -->

| Item | Type | Default | Meaning |
|---|---|---|---|
| `max_lines` | an integer of 1 or more | `1000` | How many lines the log keeps. |

<!-- /generated -->

## Programs

<!-- generated:programs -->

| Item | Type | Default | Meaning |
|---|---|---|---|
| `editor` | a string | `"notepad {path}"` on Windows<br>`"open -t {path}"` on macOS<br>`"xdg-open {path}"` on Linux | The command line that opens a file for editing, used by `edit`, `new_text`, `preview_edit`, and `edit_config`. It follows the rules of [external commands](#commands), with `{path}`, the file, as its only placeholder. |
| `use_login_shell_env` | true or false | `true` | macOS and Linux: give the programs wafu starts the environment of your login shell. |

<!-- /generated -->

With `use_login_shell_env`, wafu runs your shell once at startup as an interactive login shell and takes its environment, so a change to this item applies at the next start. While the shell runs, `WAFU_RESOLVING_ENVIRONMENT` is set to `1`, so startup files can skip what should not run there:

```sh
[ -z "$WAFU_RESOLVING_ENVIRONMENT" ] && exec tmux
```

## Fonts

`[font]` sets the font for the whole window:

<!-- generated:font -->

| Item | Type | Default | Meaning |
|---|---|---|---|
| `family` | a string | `""` | A font family name. Empty means the monospaced font of the OS (Consolas on Windows, Menlo on macOS, DejaVu Sans Mono on Linux). |
| `size` | an integer from 6 to 72 | `12` | The font size, in pixels. |

<!-- /generated -->

Each area can have its own font in a table with the same items: `[font.path]`, `[font.filelist]`, `[font.info]`, `[font.log]`, `[font.status]`, `[font.preview]`, and `[font.popup]`. What an area leaves out follows `[font]`.

```toml
[font.path]
family = "Consolas"
size = 16
```

## Colors

<!-- generated:colors -->

| Item | Type | Default | Meaning |
|---|---|---|---|
| `theme` | auto, light, or dark | `"auto"` | Which colors are used: `"auto"` (follow the OS), `"light"`, or `"dark"`. |
| `hidden_opacity` | a number from 0 to 1 | `0.7` | How opaque the names of hidden items are: `0` is invisible, and `1` is like other items. |

<!-- /generated -->

`[colors.light]` is used with the light theme and `[colors.dark]` with the dark one. Each color is written `"#rrggbb"` or `"#rgb"`.

<!-- generated:colors.themes -->

| Item | Default (light) | Default (dark) | Used for |
|---|---|---|---|
| `fg` | `"#1e1e1e"` | `"#d4d4d4"` | Text: regular files and the rest of the window's text |
| `bg` | `"#ffffff"` | `"#1e1e1e"` | Background |
| `line_number` | `"#626262"` | `"#9d9d9d"` | Line numbers in the text preview, and offsets in its binary mode |
| `mark` | `"#dcdcdc"` | `"#3c3c3c"` | Background of marked rows and of the preview of a marked image |
| `cursor` | `"#1e1e1e"` | `"#d4d4d4"` | The cursor line |
| `cursor_inactive` | `"#7d7d7d"` | `"#888888"` | The cursor line while the window is not focused |
| `dir` | `"#1f5fbf"` | `"#6fa8ff"` | Directories |
| `link` | `"#0e7c86"` | `"#4fd1e0"` | Symbolic links whose target exists |
| `link_broken` | `"#c0392b"` | `"#ff6b6b"` | Symbolic links whose target does not exist |
| `exec` | `"#2a8c2a"` | `"#5fd75f"` | Executable files |
| `special` | `"#b8860b"` | `"#f0c050"` | Named pipes and devices |
| `socket` | `"#b03a9a"` | `"#e58ae0"` | Sockets |

<!-- /generated -->

`[colors.light.extensions]` and `[colors.dark.extensions]` give regular files a color by extension, in place of `fg`. A key is the part of the name after the last dot, in either case.

```toml
[colors.light.extensions]
zip = "#a02020"
7z = "#a02020"

[colors.dark.extensions]
zip = "#ff8080"
7z = "#ff8080"
```

## Bookmarks

```toml
[[bookmarks]]
name = "Downloads"
path = "~/Downloads"
key = "D"

[[bookmarks]]
name = "Projects"
path = "~/projects"
```

`key` is optional: a digit or a letter that picks the bookmark at once in the bookmarks popup.

## Commands

`[commands]` gives a name to an external command line. Bind it to a key in `[keys.filelist]` as `"$NAME"`.

```toml
[commands]
open_terminal = "wt -d {dir}"      # Windows Terminal

[keys.filelist]
"F2" = "$open_terminal"
```

The command runs without a shell, in the directory of the active pane. The line is split into arguments as `sh` does on macOS and Linux (`'...'`, `"..."`, and `\`) and as the C runtime does on Windows (`"..."`, with `\"` for a quote). Nothing is expanded: `$VAR`, `~`, and wildcards are passed as written.

| Placeholder | Becomes |
|---|---|
| `{cursor_path}` | The item under the cursor (on the `..` row, the current directory) |
| `{marked_paths}` | The marked items |
| `{paths}` | The marked items, or the item under the cursor when nothing is marked |
| `{dir}` | The directory of the active pane |
| `{other_dir}` | The directory of the other pane |
| `{{`, `}}` | A literal brace |

Paths are full paths, and a placeholder can stand anywhere in an argument, as in `--dir={dir}`. An argument that holds `{marked_paths}` or `{paths}` is repeated for each path: `--add={paths}` becomes `--add=/a --add=/b`.

Inside an archive, the items that `{cursor_path}`, `{marked_paths}`, or `{paths}` name are first extracted to a temporary directory.

The lines typed into the dialogs of `run` and `run_command_line` follow the same rules.

## Alternative open

`open_alternative` (`Shift+Enter` in the file list) opens the item under the cursor in a program you choose for its kind:

```toml
[open_alternative]
dir = "code {path}"

[open_alternative.extensions]
pdf = "sumatrapdf {path}"
```

`dir` is for directories, and `[open_alternative.extensions]` for regular files by extension, archives and executable files included. Anything without a rule is opened as `open` (`Enter`) does.

The lines follow the rules of [external commands](#commands), with `{path}`, the item under the cursor, as their only placeholder.

## Keys

`[keys.filelist]`, `[keys.preview.image]`, and `[keys.preview.text]` bind keys in the file list, the image preview, and the text preview. Keys you do not bind keep their defaults.

```toml
[keys.filelist]
"F4" = "edit"                # a built-in command
"F2" = "$open_terminal"      # an external command from [commands]
"Q" = "nop"                  # unbind the key
"Ctrl+Q" = ""                # give the key back its default
```

[commands.md](commands.md) lists every command with its default keys. The command palette (`?`) lists the commands available where you are, with the keys bound to them.

A key is written as modifiers and a key name joined by `+`, as in `"Ctrl+Shift+Left"`. Case does not matter.

- Modifiers: `Ctrl` (the Control key on macOS too), `Alt`, and `Shift`.
- Letters: `A` to `Z`. Write `"Shift+A"` for the capital.
- Digits and symbols: the character as typed, without `Shift`: `"!"`, not `"Shift+1"`.
- Named keys: `Enter`, `Space`, `Tab`, `Backspace`, `Esc`, `Up`, `Down`, `Left`, `Right`, `PageUp`, `PageDown`, `Home`, `End`, `Insert`, `Delete`, and `F1` to `F12`.
