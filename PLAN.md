# Plan

## REPL input editing

Improve input editing so users can edit text at any position, not only at the end.

### First priority

- [x] Move the cursor with Left and Right.
- [x] Move to the start or end of the line with Home / End and Ctrl+A / Ctrl+E.
- [x] Move one word at a time with terminal-supported shortcuts, such as Ctrl+Left / Ctrl+Right or Option+Left / Option+Right on macOS.
- [x] Insert text at the cursor.
- [x] Delete text before or after the cursor with Backspace / Delete.
- [x] Delete the previous word with Ctrl+W.
- [x] Delete from the cursor to the start or end of the line with Ctrl+U / Ctrl+K.
- [x] Restore the last deleted text with Ctrl+Y.
- [x] Preserve terminal copy, paste, and text selection controls.
- [x] Handle paste safely: pasted newlines must not submit the prompt.
- [x] Recall previous prompts with Up / Down.

Implementation notes:

- Ctrl+Y restores the last nonempty deletion made with Ctrl+W, Ctrl+U, or Ctrl+K.
- Word movement uses whitespace as the boundary. Alt+B / Alt+F also work. Option shortcuts require the terminal to send Alt-arrow or Escape+B / Escape+F sequences.
- Paste safety requires terminal support for bracketed paste. Pasted line breaks and tabs become spaces. Terminal control characters are removed. Without bracketed paste, the terminal does not distinguish pasted Enter from a typed Enter.
- Copy and paste shortcuts remain terminal controls. The REPL does not enable mouse capture.
- Down restores the input and cursor position saved before history navigation.
- Validation: 65 Rust tests passed; Clippy passed with warnings denied. A pseudo-terminal test passed for bracketed paste, pasted newline safety, Left, insertion, Ctrl+A, Escape, and paste-mode cleanup.

### Further improvements

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
