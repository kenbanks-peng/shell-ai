# REPL input editing

The Rust terminal session supplies these controls for all supported shells.
No extra shell configuration is required.

| Key | Action |
| --- | --- |
| Left / Right | Move one Unicode text character. |
| Ctrl+Left / Ctrl+Right, or Alt+B / Alt+F | Move one word. |
| Home / End, or Ctrl+A / Ctrl+E | Move to the start or end of the prompt. |
| Backspace / Delete | Delete before or after the cursor. |
| Ctrl+W / Ctrl+U / Ctrl+K | Cut the previous word, text before the cursor, or text after the cursor. |
| Ctrl+Y | Insert the last cut text. |
| Ctrl+_ or Alt+U | Undo an edit. |
| Alt+R | Redo an edit. |
| Ctrl+R | Search prompt history. |
| Alt+E | Open the prompt in `$EDITOR`. |
| Tab | Complete a path or command at the cursor. |
| Up / Down | Select a history entry or return to the current draft. |
| Escape | Clear the prompt. If it is empty, close the session. |
| Ctrl+C | Close the session without a provider request. |
| Enter | Submit the prompt. |

## Undo and redo

Undo restores the text and cursor position. Paste, completion, editor output,
and history selection can each be undone. A new text change clears the redo
list. Cursor movement does not clear it. The session keeps up to 1,000 undo
steps. This list is not saved when the session closes.

## History search

Press Ctrl+R, then type search text. The search uses a case-sensitive substring
match, with the newest match first. Press Ctrl+R again for an older match. After
the last match, another Ctrl+R starts again from the newest match.

Press Enter to accept the match without submitting it. Press Escape or Ctrl+C
to leave search and restore the draft and its cursor. Enter with no match also
restores the draft. Search does not add entries to history.

The configured history key still opens the existing history picker from an
empty prompt.

## Long prompts

Long prompts wrap within the terminal width. The session reserves rows as
needed. If the prompt is taller than the terminal, only the part around the
cursor is shown. A terminal resize updates the display. Wide characters,
combining marks, and emoji are measured as Unicode text characters.

Up and Down still select history entries. Left and Right move through wrapped
text. Prompts remain a single logical line.

## External editor

Set `EDITOR` to an executable, with optional arguments. Quoted executable paths
and arguments are supported. For example, `EDITOR='code --wait'` waits for that
editor to close the file. The editor command is not evaluated by a shell.

Alt+E writes the prompt to a private temporary file. The editor uses the
controlling terminal with normal terminal input enabled. Close the editor to
return to the session. The temporary file is then removed.

An editor failure leaves the prompt unchanged and shows an error. Edited line
breaks become spaces, and terminal control characters are removed. Editor
output does not submit the prompt.

## Completion

Tab completes the current whitespace-delimited token. It checks local paths
and executable files in `PATH`. Relative paths, absolute paths, and `~/` paths
are supported. A directory completion ends with `/`.

With multiple matches, Tab inserts their common prefix. If there is no longer
common prefix, the prompt does not change. Completion does not execute a
command or expand shell expressions. Quoted tokens, variables, and backslash
escapes are not completed. Text after the cursor is kept.

## Terminal shortcuts

The session uses standard terminal key sequences. It does not enable a special
keyboard protocol. A terminal can intercept a shortcut before the session
receives it. If Ctrl+_ is not available, use Alt+U. Configure Option as Alt if
required by your terminal. Ctrl+Z is not used for undo.

Terminal copy and paste combinations, such as Ctrl+Shift+C and Ctrl+Shift+V,
are not assigned to input actions. Bracketed paste inserts text without
executing embedded key actions.

## Checks

Run the Rust checks:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

On Unix with Zsh installed, use the PTY checks to test the real key decoder,
wrapping, terminal resize, editor return, and completion. They also test the
`?` launch binding, captured command output, and terminal mode restoration
after a cursor-query timeout. They use the Python standard library and a local
mock provider. They do not contact external providers:

```sh
cargo build
python3 tests/terminal_smoke.py
```
