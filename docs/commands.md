<!-- Generated from the command table of wafu-core. Do not edit by hand; regenerate with: WAFU_UPDATE_DOCS=1 cargo test -p wafu-core --test commands_doc -->

# Commands

To bind keys to these commands, see [Keys](configuration.md#keys). A command works only in the file list unless its section or its description says otherwise.

## Opening

| Command | Default keys | Description |
|---|---|---|
| `open` | `Enter` | Open the item under the cursor: go into a directory or archive, run a program, or preview any other file |
| `open_alternative` | `Shift+Enter` | Open the item under the cursor with the program configured for its kind or extension |
| `open_with_os` | `Ctrl+Enter` | Open the item under the cursor with the OS's default application |
| `open_as_text` | `V` | Preview the file under the cursor as text, whatever its type |

## Navigation

| Command | Default keys | Description |
|---|---|---|
| `go_parent` | `BS` | Go to the parent directory |
| `go_root` | `/` | Go to the root directory, or to the root of the archive the active pane is in |
| `go_home` | `~` | Go to the home directory |
| `go_to_path` | `Shift+J` | Go to a typed path |
| `browse` | `T` | Go to a directory picked in the OS's directory dialog |
| `find` | `Shift+F` | Find the items below the active pane's directory that match a glob pattern and go to the one picked |
| `bookmarks` | `J` | Go to the bookmark picked from the list |
| `history` | `H` | Go to a directory picked from the history of the active or the other pane |
| `go_back` | `Ctrl+Shift+Left` | Go back in the active pane's history |
| `go_forward` | `Ctrl+Shift+Right` | Go forward in the active pane's history |
| `drives` | `L` | Go to the drive picked from the list (Windows only) |
| `go_prev_drive` | `Ctrl+Left` | Go to the previous drive, wrapping around (Windows only) |
| `go_next_drive` | `Ctrl+Right` | Go to the next drive, wrapping around (Windows only) |

## Panes

| Command | Default keys | Description |
|---|---|---|
| `focus_left` | `Left` | Make the left pane active |
| `focus_right` | `Right` | Make the right pane active |
| `focus_other` | `Tab` | Make the other pane active |
| `sync_to_other` | `O` | Show the other pane's directory in the active pane |
| `sync_from_other` | `Shift+O` | Show the active pane's directory in the other pane |
| `swap_panes` | `Alt+O` | Swap the directories of the two panes |

## Cursor and marks

These commands also work in the image preview.

| Command | Default keys | Description |
|---|---|---|
| `cursor_up` | `Up` | Move the cursor up one row, wrapping to the bottom |
| `cursor_down` | `Down` | Move the cursor down one row, wrapping to the top |
| `cursor_page_up` | `PageUp` | Move the cursor up one page |
| `cursor_page_down` | `PageDown` | Move the cursor down one page |
| `cursor_top` | `Home` | Move the cursor to the first row |
| `cursor_bottom` | `End` | Move the cursor to the last row |
| `search` | `F` | Move the cursor to an item by typing part of its name |
| `jump_a` to `jump_z`, `jump_0` to `jump_9` | `Ctrl+A` to `Ctrl+Z`, `Ctrl+0` to `Ctrl+9` | Move the cursor to the next item whose name starts with the command's letter or digit, wrapping around |
| `mark_down` | `Space` | Toggle the mark on the item under the cursor and move the cursor down |
| `mark_up` | `Shift+Space` | Toggle the mark on the item under the cursor and move the cursor up |
| `mark_range_above` | `Ctrl+Space` | Mark the items from the nearest mark above the cursor down to the cursor |
| `mark_range_below` | `Ctrl+Shift+Space` | Mark the items from the cursor down to the nearest mark below it |
| `invert_marks` | `A` | Invert the marks on the files, leaving directories as they are |
| `invert_marks_all` | `Shift+A` | Invert the marks on the files and the directories |
| `mark_glob` | `@` | Mark the files whose names match a glob pattern |
| `mark_compare` | `W` | Mark the items missing from the other pane, or also the newer or different ones (file list only) |
| `cursor_next_mark` | `Ctrl+:` | Move the cursor to the next marked item, wrapping around |
| `cursor_prev_mark` | `Ctrl+*` | Move the cursor to the previous marked item, wrapping around |
| `clear_marks` | `` ` `` | Unmark every item in the active pane |

## View

| Command | Default keys | Description |
|---|---|---|
| `sort` | `S` | Sort the active pane by name, extension, size, or time |
| `toggle_hidden` | `.` | Show or hide hidden items in the active pane |
| `filter` | `:` | Show only the files in the active pane whose names match a glob pattern |
| `clear_filter` | `*` | Remove the active pane's filter |
| `reload` | `,` | Read the active pane's directory again |
| `split_left` | `Alt+Left` | Move the border between the panes to the left (file list and image preview) |
| `split_right` | `Alt+Right` | Move the border between the panes to the right (file list and image preview) |

## Log

These commands work everywhere: in the file list and in both previews.

| Command | Default keys | Description |
|---|---|---|
| `log_shrink` | `Alt+Down` | Make the log one line shorter |
| `log_grow` | `Alt+Up` | Make the log one line taller |
| `log_scroll_up` | `Shift+Up` | Scroll the log up one line |
| `log_scroll_down` | `Shift+Down` | Scroll the log down one line |
| `log_page_up` | `Shift+PageUp` | Scroll the log up one page |
| `log_page_down` | `Shift+PageDown` | Scroll the log down one page |
| `log_top` | `Shift+Home` | Scroll to the top of the log |
| `log_bottom` | `Shift+End` | Scroll to the bottom of the log |
| `log_clear` | `Alt+D` | Clear the log |
| `log_copy` | `Alt+Y` | Copy the log to the clipboard |

## Preview

These commands work only in the text preview.

| Command | Default keys | Description |
|---|---|---|
| `preview_scroll_up` | `Up` | Scroll the text preview up one line |
| `preview_scroll_down` | `Down` | Scroll the text preview down one line |
| `preview_page_up` | `PageUp` | Scroll the text preview up one page |
| `preview_page_down` | `PageDown` | Scroll the text preview down one page |
| `preview_top` | `Home` | Scroll to the top of the text preview |
| `preview_bottom` | `End` | Scroll to the bottom of the text preview |
| `preview_toggle_mode` | `Tab` | Switch the text preview between text and a hex dump |
| `preview_encodings` | `C` | Decode the text preview with the encoding picked from the list |
| `preview_edit` | `E` | Close the text preview and open its file in the editor |
| `preview_close` | `Enter`, `Esc` | Close the preview (both previews) |

## Files

| Command | Default keys | Description |
|---|---|---|
| `copy` | `C` | Copy the marked items to the other pane |
| `move` | `M` | Move the marked items to the other pane |
| `delete` | `D` | Delete the marked items permanently |
| `trash` | `Shift+D` | Move the marked items to the trash |
| `rename` | `R` | Rename the item under the cursor |
| `rename_replace` | `Shift+R` | Rename the marked items or the item under the cursor by find and replace |
| `rename_template` | `Ctrl+Shift+R` | Rename the marked items or the item under the cursor from a template |
| `new_dir` | `K` | Create a directory in the active pane and go to it |
| `new_file` | `Shift+K` | Create an empty file in the active pane |
| `new_text` | `Shift+E` | Create a file in the active pane and open it in the editor |
| `edit` | `E` | Open the file under the cursor in the editor |
| `run` | `X` | Run a command line that starts out with the path of the item under the cursor |
| `run_command_line` | `Shift+X` | Run a command line that starts out empty |
| `copy_path` | `Shift+Y` | Copy the paths of the marked items or the item under the cursor to the clipboard |
| `copy_name` | `Y` | Copy the names of the marked items or the item under the cursor to the clipboard |
| `copy_files` | `Alt+C` | Put the marked items or the item under the cursor on the clipboard as files |
| `paste_files` | `Alt+V` | Copy the files on the clipboard into the active pane |
| `shell_menu` | `\` | Run a shell menu command on the marked items or the item under the cursor (Windows only) |
| `calculate_size` | `Shift+I` | Log the total size of the marked items or the item under the cursor |
| `calculate_size_on_disk` | `I` | Log the disk space taken by the marked items or the item under the cursor |

## Archives

| Command | Default keys | Description |
|---|---|---|
| `open_as_archive` | `Shift+V` | Open the file under the cursor as an archive, whatever its extension |
| `extract` | `U` | Extract the marked archives or the archive under the cursor into the other pane |
| `extract_here` | `Shift+U` | Extract the marked archives or the archive under the cursor into the active pane |
| `compress` | `P` | Compress the marked items or the item under the cursor into an archive in the other pane |
| `compress_here` | `Shift+P` | Compress the marked items or the item under the cursor into an archive in the active pane |

## Application

These commands work everywhere: in the file list and in both previews.

| Command | Default keys | Description |
|---|---|---|
| `palette` | `?` | Run the command picked from a list of those available here |
| `edit_config` | `Z` (file list) | Open the configuration file in the editor |
| `new_window` | `Alt+Z` | Open a new window showing the same directories |
| `new_window_as_admin` | `Shift+Alt+Z` | Open a new window showing the same directories, as administrator (Windows only) |
| `quit` | `Q` (file list) | Quit, saving the state for the next start |
| `quit_without_saving` | `Shift+Q` (file list) | Quit without saving the state |
| `nop` |  | Do nothing, which unbinds the key bound to it |
