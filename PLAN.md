# Plan

## REPL input editing

### Remaining improvements

- [x] Support undo and redo.
- [x] Search prompt history with Ctrl+R.
- [x] Wrap long prompts and keep cursor movement correct.
- [x] Open the current prompt in `$EDITOR`.
- [x] Complete paths and commands with Tab.

See [REPL input editing](docs/input-editing.md) for controls and checks.

### Design constraints

- Keep input editing portable across supported shells.
- Do not require shell-specific features outside the generated shell integration.
- Check shortcut support in the terminal. Preserve terminal controls where possible.
