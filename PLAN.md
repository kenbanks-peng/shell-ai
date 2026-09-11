# Plan

## REPL input editing

### Remaining improvements

- [ ] Support undo and redo.
- [ ] Search prompt history with Ctrl+R.
- [ ] Wrap long prompts and keep cursor movement correct.
- [ ] Support multiline input with a separate shortcut to insert a newline without submission.
- [ ] Open the current prompt in `$EDITOR`.
- [ ] Complete paths and commands with Tab.
- [ ] Clear the current input with Ctrl+C without closing the REPL.

### Design constraints

- Keep input editing portable across supported shells.
- Do not require shell-specific features outside the generated shell integration.
- Check shortcut support in the terminal. Preserve terminal controls where possible.
