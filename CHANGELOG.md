# wafu

## 0.5.0

### Minor Changes

- Add `copy_files` (`Alt+C`) and `paste_files` (`Alt+V`), which copy files through the OS clipboard: `copy_files` puts the marked items, or the item under the cursor, on the clipboard as files, and `paste_files` copies the files on the clipboard into the active pane's directory.

- Command names are made consistent, which gives twenty commands new names. Key bindings under the old names are reported as unknown and ignored. The popup of `new_dir` is now titled `New directory`, and the popup of `preview_encodings` is titled `Encodings`. A command line that cannot be parsed in the popup of `run` or `run_command_line` is logged as `Cannot run command line: …`, without the command name.

  | Old name | New name |
  |---|---|
  | `enter_as_archive` | `open_as_archive` |
  | `history_back` | `go_back` |
  | `history_forward` | `go_forward` |
  | `drive_prev` | `go_prev_drive` |
  | `drive_next` | `go_next_drive` |
  | `switch_pane` | `focus_other` |
  | `sync_other_to_this` | `sync_from_other` |
  | `page_up` | `cursor_page_up` |
  | `page_down` | `cursor_page_down` |
  | `next_mark` | `cursor_next_mark` |
  | `prev_mark` | `cursor_prev_mark` |
  | `pane_shrink_left` | `split_left` |
  | `pane_grow_left` | `split_right` |
  | `clear_log` | `log_clear` |
  | `copy_log` | `log_copy` |
  | `preview_encoding` | `preview_encodings` |
  | `close_preview` | `preview_close` |
  | `mkdir` | `new_dir` |
  | `run_file` | `run` |
  | `run_command` | `run_command_line` |

- A place inside an archive is now written from a root named after the archive, as `a.zip:/path/to/file`, in the pane header, the window title, the log, popups, and the copied path. The directory holding the archive is no longer part of it, and Windows no longer mixes `\` and `/`.

- The `rename_template` placeholder for the name without its extension is now `{stem}` instead of `{name}`, so that "name" means the whole name everywhere.

- Placeholders in command lines are now replaced anywhere in an argument (`--dir={dir}`), an argument that holds `{paths}` or `{marked_paths}` is repeated for each path, a brace meant as text must be doubled (`{{`, `}}`) while any other brace or unknown name is an error, and a command whose placeholder has no value is not started. `editor` and `[open_alternative]` now take `{path}`, the file they open, as their only placeholder, in place of `{paths}` and `{cursor_path}`.

- An external command can now be written as a plain string, `NAME = "..."` under `[commands]`, as well as a table with `command_line`. An empty string in either form now takes back a command defined by a file below, instead of being reported as invalid.

- A command line can now use only one of `{cursor_path}`, `{marked_paths}`, and `{paths}`; a line that mixes them is an error. A command now checks only the items its line uses: an item it does not use no longer keeps it from starting because of a name ending with a dot or a space on Windows, or because it is a link inside an archive. Inside an archive, a command extracts only those items, and a line with `{cursor_path}` now starts on a `..` row while items are marked.

- The names of configuration items are made consistent, which renames the items below. An item under an old name is reported as unknown and ignored; `[keys.image]` and `[keys.text]` are ignored as a whole, with every binding in them. A command table that still has `run` is reported as missing `command_line` and is left undefined, so the keys bound to it do nothing.

  | Old name | New name |
  |---|---|
  | `[keys.image]` | `[keys.preview.image]` |
  | `[keys.text]` | `[keys.preview.text]` |
  | `mkdir_enters_dir` | `new_dir_goes_to_dir` |
  | `search_matches_anywhere = true` / `false` | `search_match = "anywhere"` / `"start"` |
  | `compare_tolerance_ms` | `mark_compare_tolerance_ms` |
  | `max_find_results` | `max_find_matches` |
  | `from_left`, `from_right` under `[preview.image]` | `placement_from_left`, `placement_from_right` |
  | `fifo` under `[colors.light]` and `[colors.dark]` | `special` |
  | `broken` under `[colors.light]` and `[colors.dark]` | `link_broken` |
  | `run` in a command written as a table | `command_line` |

- The window title now puts the path first and shows the version, as `<path> - wafu <version>`, so that a taskbar that cuts a long title keeps the path.

- The log now starts with a line that shows the version, `Started wafu <version>`.

- Add the `extract_here` command (`Shift+U`), which extracts archives into the directory of the active pane instead of the other pane.

- The default keys no longer use function keys, and modifiers now follow one pattern. Five commands move to other keys, and three get a default key for the first time.

  | Command | Old key | New key |
  |---|---|---|
  | `palette` | `F1` | `?` |
  | `reload` | `F5` | `,` |
  | `rename_template` | `Alt+R` | `Ctrl+Shift+R` |
  | `open_as_archive` | `Shift+U` | `Shift+V` |
  | `new_file` | `N` | `Shift+K` |
  | `swap_panes` | none | `Alt+O` |
  | `log_clear` | none | `Alt+D` |
  | `log_copy` | none | `Alt+Y` |

- Add the `compress_here` command (`Shift+P`), which creates the archive in the directory of the active pane instead of the other pane.

- Make `open_alternative` (`Shift+Enter`) on the `..` row act on the current directory itself, or on the archive file at the root of an archive, as `open_with_os` does. Without a rule for it, the `..` row still moves up.

- `docs/commands.md` now lists every command with its default keys, where it works, and a one-line description. The command palette shows the description of the selected built-in command in the status bar.

- The history, drive, and encoding popups now mark the current item with `*`, like the sort popup. The encoding popup no longer has the `Auto-detect` item; a preview goes back to the detected encoding when it is opened again.

- On Windows and macOS, files can now be dragged from the file list to other applications, such as Explorer, Finder, or a browser's upload field: dragging a row hands over the marked items, or the item dragged when nothing is marked, as a copy.

- The Backspace key is now written `Backspace` in key bindings and shown so in the command palette. A binding written with `BS` is reported as an invalid key and ignored. Keys with several modifiers are shown as `Ctrl`, `Alt`, `Shift`, in that order, so `Shift+Alt+Z` now reads `Alt+Shift+Z`; a binding may still write them in any order.

### Patch Changes

- Extracting or copying out of an archive now skips an entry whose name is too long for the destination and goes on with the rest. It used to stop the whole operation.

- Pressing Tab in the history popup to show the other pane's history now keeps the cursor row on screen. It could be scrolled out of view when both histories had the same length and the same current place in them.

- Files extracted or opened from a downloaded archive now carry its mark of origin (the quarantine attribute on macOS, the zone identifier on Windows), so the OS runs its usual first-open checks on them.

- Confirming quit a second time, while the operation cancelled by the first has not stopped, now quits without waiting, so an operation stuck on an unresponsive drive no longer keeps the window from closing.

- The file list now always lines its rows up from the top of the list. At the end of a directory the last row was placed against the bottom edge instead, and it could be cut off after the pane was made shorter.

- An empty `extends` (`extends = ""`) now extends nothing, like an empty string for other string items, instead of making wafu ignore the whole configuration. An empty string inside an `extends` array is reported as invalid and skipped.

- A new file, a new text file, or the archive to compress to must now be given a single name, and `new_dir` makes directories only below the current one. Extracting into directories now fails for an archive whose name cannot be a directory there (such as `...zip`), and on Windows a name that Windows does not allow is refused with the reason instead of being tried.

- On macOS, quitting with Cmd+Q or the application menu's Quit now saves the state (the panes' directories, histories, and view settings, and the window frame), and asks for confirmation while an operation is running. It used to end wafu at once without saving anything.

- Fix overwriting a file or link with itself deleting it when it cannot be read, and a FIFO freezing an overwrite or a rename. Moving a directory onto itself through a link or another spelling of the path now fails instead of removing its empty directories.

- Scrolling the file list with the mouse wheel or the scrollbar is no longer undone when the directory reloads or the window height changes, and the list no longer stays scrolled past its end after the window grows taller.

- Extracting an archive no longer changes the permissions or modification time of directories that already existed at the destination, and the permissions of extracted files and directories now respect the umask.

- On Windows, commands now refuse a selected item whose name ends with a dot or a space and log why, instead of acting on a different item whose name lacks that ending.

- Fix a pane retrying without end, or starting with no location, when the nearest existing directory cannot be listed. The pane now moves up to a directory it can read, or to the home directory.

- The image preview now refuses an image that would need more than 1 GiB of memory to decode, instead of trying to allocate it. A small file that declares huge dimensions could otherwise freeze or crash wafu.

- The note of `rename_template` now quotes an unknown placeholder, as the log does: `Unknown placeholder "{x}"`.

- Fix extraction stopping at a zip symbolic link whose target is not UTF-8, and skip a link whose target is longer than 1 MiB, without reading it into memory when it is in a zip. On Windows, extraction no longer connects to a network host named by a link target; a link is made as a directory link only when its target is a relative path to a directory with no other link on the way.

- Going back or forward through the history now skips a place that cannot be read, such as an unplugged drive, and goes on to the next one, instead of staying where it is. The skipped place stays in the history.

- On Windows, a program written without its extension, such as `code` for `code.cmd`, now starts instead of failing with "program not found".

- Name the command in the log when `edit`, `open_with_os`, `open_alternative`, `run`, or an external command inside an archive ends without starting: `Cannot edit …: 1 entry was skipped` and `Edit cancelled`, where the lines spoke of an extraction.

- Overwriting a file during a copy, move, compression, or extraction no longer deletes the existing one first, so it is kept when the operation fails or is cancelled.

- A pane's location is now normalized however it is reached, so going up from a location given with `..` works, Windows locations always use `\`, and `D:` goes to `D:\`. A relative bookmark path is now taken from the directory of the configuration file it is written in.

- A key in `keys` written as a space or a control character (such as `" "`, `"\t"` or `"Ctrl+ "`) is now reported as invalid, since it can never match a key press. Write `Space` or `Tab` instead.

- Skip symbolic links when extracting a directory inside an archive for another program (`open_with_os`, `edit`, `run`, external commands). The program is not started, since a link could lead it to a file outside the archive.

- Fix `edit`, `open_with_os`, `run`, and external commands following a symbolic link inside an archive to the file it points to outside the archive. They now do nothing for such a link and log the reason.

- Fix compressing over one of the files being compressed deleting that file. Renaming the output onto a source in the conflict popup, or a default name that differs from a source only in case, now fails instead.

- Make the wording of popups uniform: the batch rename popups are titled `Rename (replace)` and `Rename (template)`, the popups of `mark_glob` and `mark_compare` are titled `Mark (glob)` and `Mark (compare)`, the delete confirmation shows just the name or the number of items, the find popups no longer show the directory, and a wrong archive password is reported under the field.

- Log the reason when `open`, `open_alternative`, `open_as_text`, or `open_as_archive` does nothing on a symbolic link inside an archive.

- On Windows, the window now reopens where it was when monitors have different scale factors, and opens where the system puts it when the saved position is no longer on an attached screen, as after unplugging a monitor.

- Compressing now fails at a file name or link target that is not valid UTF-8 instead of replacing the invalid characters, which could give two files the same name in the archive.

- A name that ends in a dot, such as `report.`, is now shown with the dot. It used to be shown as if the dot were not there.

## 0.4.0

### Minor Changes

- Bind `N` to `new_file`, `Y` to `copy_name`, and `Shift+Y` to `copy_path` in the file list by default.

- Popups now act on a key only when Ctrl, Shift, and Alt are all up, as the file list does. Held with a modifier, Enter, Esc, Space, Tab, the arrows, and the letter or digit of a row do nothing, and Esc does not cancel a file operation or a move; in an input field the arrows are left to the field. Shift+Tab still moves between the fields of a popup.

- Enter on a checkbox in the sort popup now applies the sort settings as they are and closes the popup instead of flipping the checkbox. In the conflict popup, Enter on "Apply to all" no longer flips it; use Space or its key.

- Popups are wider, and their messages and notes wrap instead of being cut off. While a popup with a list is open, the status bar shows the full text of the selected row.

- The file list is narrower by default: the size column holds 9 digits and writes larger sizes like `1.2G`, the extension column holds 4 characters, and the gap between columns follows the font size. New `[filelist]` settings `size_digits`, `extension_chars`, and `min_name_chars` set the widths of the size, extension, and name columns.

- In the bookmarks popup, the status bar shows the path of the selected bookmark after its name; in the command palette, it shows the command line of the selected external command. Both appear as written in the configuration.

- Bind `` ` `` to `clear_marks` in the file list and the image preview by default.

- Add the `mark_compare` command (`W`), which marks the files in the active pane that are missing from the other pane, or also the newer or different ones. The new `filelist.compare_tolerance_ms` option sets how far apart two modification times may be and still count as the same.

- Filter and glob-mark patterns can exclude names with `~`, as in `*~*.bak`. A literal `~` is now written `\~` or `[~]`, `\` is an escape on Windows too, and spaces around a pattern are no longer trimmed.

- New `[filelist]` settings `time_shows_full_year` and `time_shows_seconds` narrow the file list's time column: when `false`, the year is written with its last two digits and the seconds are left out. The status bar still shows the full time.

- The number of places each pane keeps in its history and the number of lines the log keeps can now be set with `max_history` under `[filelist]` and `max_lines` under the new `[log]` table.

- Sizes outside the file list's size column are now always written exactly, with thousands separators, as in `1,234 bytes`; `size_format` applies only to that column. `calculate_size` and `calculate_size_on_disk` also log an approximation and list directories first, as in `Size of docs: 48,318,382,080 bytes (45GB) in 3 directories and 12 files`, and counts of one now read `1 item`, `1 byte`, and `1 match`.

- Add the `find` command (`Shift+F`), which searches below the pane's directory for entries matching a glob and goes to the one picked from the list. A glob without a slash matches names at any depth; one with a slash matches the path from the directory searched.

- wafu is now available from Homebrew on macOS and Linux (`brew install iorate/tap/wafu`) and from Scoop on Windows, where it installs without Gatekeeper or SmartScreen blocking it. Linux gets a prebuilt binary for the first time, the macOS disk image is replaced by a `.tar.gz` that Homebrew installs, and every archive now includes the licenses of the bundled dependencies.

- Add the `browse` command (`T`), which opens the directory dialog of the OS and moves the active pane to the directory picked in it.

- Long lines in the log now wrap instead of being cut off at the right edge. The log scrolls one entry at a time, and its scrollbar counts entries, however many lines each wraps to.

- The number of matches `find` collects before it stops can now be set with `max_find_results` under `[filelist]`.

- Add `max_bytes` to `[preview.text]` and `[preview.image]` to set how many bytes of a file the text preview reads and how large an image the image preview opens. The note after a cut file and the error for an image that is too large state the limit in bytes.

- Building wafu from source, such as with `cargo install`, now requires Rust 1.95 or later.

- PageUp and PageDown now move the cursor by a page in every popup list, and Home and End jump to the first and last row in the lists without a filter field.

- Add `rename_replace` (Shift+R) and `rename_template` (Alt+R), which rename the marked items, or the item under the cursor, by replacing text in their names or from a template such as `photo_{001}{ext}`, showing the new names before Enter. In that popup, Alt with Up, Down, PageUp, PageDown, Home, or End scrolls the list.

- Add a scrollbar to the text viewer, which shows the position in the document and scrolls it by dragging. Lines in the viewer wrap slightly earlier to leave room for it.

- Add the `log_top` and `log_bottom` commands (`Shift+Home` and `Shift+End`), which jump to the oldest and the newest lines of the log.

### Patch Changes

- `rename`, and renaming in a copy or move conflict, now refuse a name that is `.` or `..` or contains a path separator, instead of moving the item to another directory.

- Treat names that differ only in Unicode normalization (NFC and NFD) as the same name: the cursor lands on an entry reached by a typed path, `rename` does nothing when the name is retyped, and a case-only rename of a decomposed name works.

- The offset column of the binary preview is six digits wide instead of eight, and widens only when the preview shows more than 16 MiB.

- Fix the cursor of a long popup list sometimes showing in the middle of the list, not at its bottom edge, right after the popup opens.

- Keep at least eight characters of a file name visible in a narrow pane; the other columns are cut off at the pane's edge instead.

- The text viewer no longer scrolls past the end of the file, and the End key and paging now account for wrapped lines: End shows the last line in full, and paging neither skips nor repeats lines.

- Renaming an item to a name that differs only in case no longer replaces another item of that name. On a case-sensitive file system, renaming a symbolic link onto its target lost the target, and renaming a hard link onto another reported success without doing anything; both now fail with "already exists".

## 0.3.0

### Minor Changes

- Log messages now start with a capital letter and follow one format. An operation names the item it was run on, or counts the selected items rather than the files inside them, as in "Copied docs".

- Shift+I (calculate_size) logs the number of files and directories and the total size of the marked items, or of the item under the cursor, counting what is inside directories. Symbolic links are not followed and count as nothing.

- I (calculate_size_on_disk) logs the space the marked items, or the item under the cursor, take up on disk, counting what is inside directories. On macOS and Linux, the space of a file with several hard links is counted once.

- The image viewer shows the "Loading" popup while an image takes more than a moment to load, as the text viewer does. Moving to another image before it appears starts the wait over, so that it stays out of the way while you move through images quickly.

- Regular files can be colored by extension with the new `[colors.light.extensions]` and `[colors.dark.extensions]` tables, as in `zip = "#a02020"`. Directories, executables, links, and other kinds keep their own color.

- `history` (H) now narrows its list as you type, like the command palette: to the paths that contain the text, ignoring case. The 0-9 and A-Z keys no longer pick a place in the history at once.

- The previews no longer show the encoding, the image format, and whether the image is marked in a corner of the window. Instead, the preview of a marked image has the background color of marked rows.

- Add the `open_alternative` command (`Shift+Enter` in the file list), which opens the item under the cursor in the program set for directories or for its extension in `[open_alternative]`, and does what `open` does when none is set.

- The configuration file created on first launch now contains only a link to the new configuration reference, docs/configuration.md. Existing configuration files are not changed.

- `search` (F) now finds the text anywhere in a name, not only at its start. Set `search_matches_anywhere = false` in `[filelist]` to match only the start as before.

- An empty string in the configuration now means the item is not set, so a configuration file can take back what a file it extends set, such as `pdf = ""` for an `open_alternative` rule. In a key table, `""` restores the built-in binding.

- A configuration file can name other files as its base with `extends`, so that settings shared between machines live in one file and each machine keeps only its differences. The per-OS `[os.*]` sections are removed in its favor.

- The filter now stays when the pane moves to another directory or archive, until it is cleared or replaced. It is not saved when wafu exits.

- Add `[preview.image]` with `from_left` and `from_right`, which choose whether an image opened from each pane is previewed in place of the other pane (`"other_pane"`) or of both panes (`"both_panes"`). `hide_log_in_preview` is replaced by `hide_log` in `[preview.text]` and in `[preview.image]`.

- Bookmarks take their key from a new `key` item in `[[bookmarks]]`: one digit or letter, such as `key = "D"`. The bookmarks popup no longer gives 0-9 and A-Z to the bookmarks in order, so a bookmark without `key` is picked with the arrow keys and Enter.

- Archives inside archives can now be entered and browsed like any other archive, and extract works on archives inside an archive, unpacking them into the other pane.

- The configuration tables are reorganized: `[general]` is split into `[filelist]` (where `mkdir_then_enter` is now `mkdir_enters_dir`) and `[programs]` (`editor`, `use_login_shell_env`), `[appearance.font]` is now `[font]`, `theme` moves to `[colors]` and `size_format` to `[filelist]`, and `cursor-inactive` is now `cursor_inactive`. Items under the old names are reported as unknown and ignored.

- The text preview's encoding can now be chosen from a searchable list with C (preview_encoding), or set back to auto-detection. The choice lasts until the preview is closed.

- In the directory history list, Tab switches between the histories of the left and right panes. A place picked from the other pane's history opens in the active pane.

- The color of line numbers in the text preview can be set with `line_number` in `[colors.light]` and `[colors.dark]`, and how faint hidden items look with `hidden_opacity` in `[colors]`. Line numbers no longer follow `fg` and `bg`: nothing changes with the default colors, but if you changed those, set `line_number` to match.

## 0.2.0

### Minor Changes

- Keys during a preview come from their own tables. The text preview's movement keys are now commands (preview_scroll_up, preview_page_down, preview_top, preview_toggle_mode, and so on), Enter and Esc close a preview through close_preview, and the log keys (Alt+Up/Down, Shift+Up/Down) work there too. The palette (F1) opens during a preview and lists the commands available in it, and Alt+Z opens a new window there as well. [keys.image] and [keys.text] each have full default bindings and no longer inherit changes made under [keys.filelist]. [keys.text] covers the binary mode as well, and the [keys.binary] section is gone.

- E during a text preview (preview_edit) closes the preview and opens the shown file in the editor. Inside an archive, the entry is extracted to a temporary directory first, as edit does. Marks are not consulted: only the file being shown is opened.

- The cursor line takes a new color, `cursor-inactive`, while the window is not focused, so that it shows whether keys reach wafu. The default is a grey between the cursor and the mark background.

- Hidden files, when shown, are drawn dimmed in the file list.

- An argument in braces that is not a known placeholder, such as {}, in editor or a [commands] entry is now passed as written instead of being reported as an error.

- Commands pick their targets by one rule: open, open_with_os, enter_as_archive, edit, and rename act on the cursor row whatever is marked, while copy, move, delete, trash, extract, compress, copy_path, copy_name, shell_menu, and the {paths} of external commands act on the marks or, without marks, on the cursor row; a right click follows the same rule instead of singling out an unmarked row. The .. row now stands for the directory itself (or the archive file at an archive root) for every command, so copy_path, compress, rename, and the rest work on it as open_with_os already did, and renaming it moves the pane to the new name.

- On Windows, .com, .bat, and .cmd files are now drawn in the `exec` color with a `*` mark, like .exe files.

- The binary preview now shows the offset of each row dimmed in its own column, as the text preview shows line numbers, and no longer wraps the text column in `|`.

- V (open_as_text) opens the file under the cursor in the text preview whatever its type, so that images and archives can be looked into as well; on the .. row at an archive root, it opens the archive file itself. Content that is not text is shown in the binary mode, as for any other file.

- During a preview, `clear_log`, `copy_log`, `quit`, `quit_without_saving`, and `edit_config` can now be bound to keys and run from the palette, and the log scrolls with the mouse wheel and its scrollbar. The image preview also accepts `search` and `jump_*`, with the same default keys as the file list (F, and Ctrl with a letter or digit).

- The divider between the panes and the one above the log can be dragged with the mouse. The panes move in the same 5% steps as the keys, and the log by whole rows. The layout keys (Alt+arrows by default) also work during an image preview.

- A file is now hidden when its name starts with a dot or it carries the hidden attribute of the OS, on every OS. On Windows this hides dot files such as .gitignore as well, and on macOS it hides what Finder hides by a flag, such as ~/Library.

- Setting `use_login_shell_env = false` in `[general]` turns off reading the login shell's environment; the setting takes effect the next time wafu starts.

- New run_file (X) and run_command (Shift+X) commands open a dialog for a command line to run without a shell, starting from the cursor item's path or empty. The line accepts the same placeholders as [commands], such as {marked_paths}. Enter on an executable file now opens this dialog instead of the text preview, and on Windows a file inside an archive counts as executable by its extension, as on disk.

- On macOS and Linux, programs started by external commands and the editor now get the environment of your login shell (such as `PATH`) when wafu is not started from a terminal. Shell startup files can check `WAFU_RESOLVING_ENVIRONMENT` to skip lines that should not run while wafu reads it.

- When an entry cannot be extracted (an unsafe or invalid name, a link that cannot be made, an unsupported entry), extraction and copies out of an archive now log the entry, skip it, and go on; the summary line shows how many entries were skipped. Before, the whole operation stopped at the first such entry. Edit and open from inside an archive do not start the program when an entry was skipped.

- On Windows, Alt+Shift+Z (new_window_as_admin) opens another window as administrator through the UAC prompt; a refusal is logged. A window running as administrator says so in its title.

- Archives in tar, tar.gz (tgz), tar.bz2 (tbz2), and tar.xz (txz) can be browsed, previewed, extracted, and made with compress, alongside zip. The format of a new archive follows the extension typed in the compress popup; only zip takes a password, and a password typed for a tar archive is refused with an error, as is a name whose extension does not name one of these formats. enter_as_archive and extract detect the format from the file's contents when the extension does not identify it.

- Command lines in the configuration (editor and [commands]) are now split by the OS's own quoting rules, so some existing lines may need rewriting. On macOS and Linux, backslashes, single quotes and a leading # work as in sh; on Windows, a backslash before a double quote escapes it (write "C:\dir\\" for a path ending in a backslash), and a line starting with a space has no program and is rejected.

### Patch Changes

- On Windows, leaving a folder that had just changed could rarely freeze the window or leave a pane loading forever. Updated the file watching library to a version that fixes this.

- Files extracted from a zip that records no Unix permissions for them, such as one made on Windows or DOS, now get the system's default permissions instead of ones made up from their DOS attributes or none at all.

- The text preview no longer breaks a line at a vertical tab, form feed, next line, line separator, or paragraph separator without numbering the extra rows, which could leave text out of reach. Those characters are now shown as ↵, and line numbers stay as editors count them.

- The text preview now leaves one character of space between the line numbers and the text.

- A [commands] entry run inside an archive no longer extracts the cursor and marked items first when its line uses none of {cursor_path}, {marked_paths}, and {paths}.

- A zip entry in a compression method wafu cannot read, such as LZMA, is now skipped and logged while the rest of the archive is extracted, instead of stopping the extraction.

- Extracting a zip whose entries start with `./`, as one made from `.` does, no longer fails on its root entry; when a zip holds the same file under two spellings, such as `a.txt` and `./a.txt`, only the later one is listed and extracted.

- Zip entries now show the NTFS modification time that 7-Zip and Windows tools write, to a fraction of a second, and an extended timestamp before 1970 no longer shows as a date in 2106.

- Line numbers in the text preview are drawn slightly darker, at the same strength as other dimmed text.

- Zip archives that wafu creates now record each modification time to the second in UTC, so odd seconds, times before 1980, and, for wafu and other tools that read extended timestamps, opening the archive in another time zone no longer shift the times.

- Text fields in popups now scroll horizontally to keep the caret in view when the text is longer than the field.

- Cancelling a popup with Esc no longer leaves the keyboard dead. This happened on the first password prompt of a session and left only the mouse working until a restart.

- File names containing line-break characters, such as a carriage return, no longer break the row they are shown in; those characters are now shown as ↵.

- On Windows, `theme = "auto"` now follows a switch between light and dark mode made by PowerToys Light Switch and other tools that announce the change only once. The setting is read from the registry instead of the notification the window receives.

- A double click on the left pane while an image is previewed in the right pane now closes the preview, as a double click on the image already does. The cursor moves to the clicked row first.

- Zip entries marked as directories only by their attributes, without a trailing slash in the name, are now listed and extracted as directories instead of empty files.

- Opening a file that is not a zip now says "not a zip archive" (or "not an archive" when the name gives no format), a broken zip says "the zip archive is damaged", and the entry is named when the damage is in one, instead of the zip library's own message; for an encrypted entry, the message also says the password may be wrong.

## 0.1.0

- Initial release.
