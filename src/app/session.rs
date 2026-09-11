//! Rust-owned terminal request session.

use std::io::{self, Write};

use anyhow::Result;
use crossterm::{
    cursor::{MoveTo, RestorePosition, SavePosition},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{Clear, ClearType, ScrollUp, disable_raw_mode, enable_raw_mode},
};
use dialoguer::{Select, theme::Theme};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The settings that affect root request interaction and rendering.
pub(crate) struct SessionConfig {
    pub(crate) prompt: String,
    pub(crate) prompt_color: Option<u8>,
    pub(crate) input_color: Option<u8>,
    pub(crate) error_color: Option<u8>,
    pub(crate) history_key: char,
    pub(crate) menu_key: char,
    pub(crate) history_style: HistoryStyle,
    pub(crate) menu_style: MenuStyle,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MenuStyle {
    pub(crate) prompt: Option<u8>,
    pub(crate) item: Option<u8>,
    pub(crate) active: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HistoryStyle {
    pub(crate) prompt: Option<u8>,
    pub(crate) input: Option<u8>,
    pub(crate) item: Option<u8>,
    pub(crate) matched: Option<u8>,
    pub(crate) active: Option<u8>,
    pub(crate) active_match: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerminalEvent {
    Character(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    WordLeft,
    WordRight,
    KillWord,
    KillStart,
    KillEnd,
    Yank,
    Undo,
    Redo,
    SearchHistory,
    EditExternal,
    Complete,
    Resize,
    Cancel,
    Paste(String),
    Up,
    Down,
    Enter,
    Escape,
}

/// Single-line input; cursor positions always lie on UTF-8 character boundaries.
#[derive(Clone, Default)]
struct InputBuffer {
    text: String,
    cursor: usize,
    killed: String,
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
}

impl InputBuffer {
    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn set(&mut self, text: &str) {
        let before = (self.text.clone(), self.cursor);
        self.text.clear();
        self.cursor = 0;
        self.insert(text);
        self.record(before);
    }

    fn record(&mut self, before: (String, usize)) {
        if before.0 != self.text {
            self.undo.push(before);
            if self.undo.len() > 1000 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    fn insert(&mut self, text: &str) {
        // Paste is data, not keys. Flatten lines and remove terminal controls.
        let text: String = text
            .replace("\r\n", "\n")
            .chars()
            .filter_map(|c| {
                if c.is_whitespace() {
                    Some(' ')
                } else if c.is_control() {
                    None
                } else {
                    Some(c)
                }
            })
            .collect();
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
    }

    fn previous(&self) -> usize {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next(&self) -> usize {
        self.cursor
            + self.text[self.cursor..]
                .graphemes(true)
                .next()
                .map_or(0, str::len)
    }

    fn word_left(&self) -> usize {
        let mut start = self.cursor;
        let mut in_word = false;
        for (i, c) in self.text[..self.cursor].char_indices().rev() {
            if in_word && c.is_whitespace() {
                break;
            }
            in_word |= !c.is_whitespace();
            start = i;
        }
        start
    }

    fn word_right(&self) -> usize {
        let mut end = self.cursor;
        let mut in_word = false;
        for (i, c) in self.text[self.cursor..].char_indices() {
            if in_word && c.is_whitespace() {
                break;
            }
            in_word |= !c.is_whitespace();
            end = self.cursor + i + c.len_utf8();
        }
        end
    }

    fn remove(&mut self, start: usize, end: usize, kill: bool) {
        if start == end {
            return;
        }
        if kill {
            self.killed = self.text[start..end].to_owned();
        }
        self.text.replace_range(start..end, "");
        self.cursor = start;
    }

    /// Returns true for edits, but not cursor movement.
    fn edit(&mut self, event: TerminalEvent) -> bool {
        if matches!(event, TerminalEvent::Undo | TerminalEvent::Redo) {
            let (source, target) = if event == TerminalEvent::Undo {
                (&mut self.undo, &mut self.redo)
            } else {
                (&mut self.redo, &mut self.undo)
            };
            if let Some((text, cursor)) = source.pop() {
                target.push((self.text.clone(), self.cursor));
                self.text = text;
                self.cursor = cursor;
                return true;
            }
            return false;
        }
        let before = (self.text.clone(), self.cursor);
        let edited = self.apply(event);
        self.record(before);
        edited
    }

    fn apply(&mut self, event: TerminalEvent) -> bool {
        use TerminalEvent::*;
        match event {
            Left => self.cursor = self.previous(),
            Right => self.cursor = self.next(),
            Home => self.cursor = 0,
            End => self.cursor = self.text.len(),
            WordLeft => self.cursor = self.word_left(),
            WordRight => self.cursor = self.word_right(),
            Character(c) => {
                self.insert(&c.to_string());
                return true;
            }
            Paste(text) => {
                self.insert(&text);
                return true;
            }
            Backspace => {
                self.remove(self.previous(), self.cursor, false);
                return true;
            }
            Delete => {
                self.remove(self.cursor, self.next(), false);
                return true;
            }
            KillWord => {
                self.remove(self.word_left(), self.cursor, true);
                return true;
            }
            KillStart => {
                self.remove(0, self.cursor, true);
                return true;
            }
            KillEnd => {
                self.remove(self.cursor, self.text.len(), true);
                return true;
            }
            Yank => {
                self.insert(&self.killed.clone());
                return true;
            }
            _ => {}
        }
        false
    }
}

/// Only bind known modifier combinations. Leave clipboard shortcuts to the terminal.
fn map_key(key: KeyEvent) -> Option<TerminalEvent> {
    use TerminalEvent::*;
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    let modifiers = key.modifiers;
    if modifiers == KeyModifiers::CONTROL {
        return match key.code {
            KeyCode::Left => Some(WordLeft),
            KeyCode::Right => Some(WordRight),
            KeyCode::Char('a') => Some(Home),
            KeyCode::Char('e') => Some(End),
            KeyCode::Char('w') => Some(KillWord),
            KeyCode::Char('u') => Some(KillStart),
            KeyCode::Char('k') => Some(KillEnd),
            KeyCode::Char('y') => Some(Yank),
            // Crossterm decodes legacy 0x1f (Ctrl+_) as Ctrl+7.
            KeyCode::Char('_') | KeyCode::Char('/') | KeyCode::Char('7') => Some(Undo),
            KeyCode::Char('r') => Some(SearchHistory),
            KeyCode::Char('c') => Some(Cancel),
            _ => None,
        };
    }
    if modifiers == KeyModifiers::ALT {
        return match key.code {
            KeyCode::Left | KeyCode::Char('b') => Some(WordLeft),
            KeyCode::Right | KeyCode::Char('f') => Some(WordRight),
            KeyCode::Char('u') => Some(Undo),
            KeyCode::Char('r') => Some(Redo),
            KeyCode::Char('e') => Some(EditExternal),
            _ => None,
        };
    }
    if !modifiers.is_empty() && modifiers != KeyModifiers::SHIFT {
        return None;
    }
    match key.code {
        KeyCode::Char(c) if !c.is_control() => Some(Character(c)),
        _ if !modifiers.is_empty() => None,
        KeyCode::Left => Some(Left),
        KeyCode::Right => Some(Right),
        KeyCode::Home => Some(Home),
        KeyCode::End => Some(End),
        KeyCode::Backspace => Some(Backspace),
        KeyCode::Delete => Some(Delete),
        KeyCode::Up => Some(Up),
        KeyCode::Down => Some(Down),
        KeyCode::Tab => Some(Complete),
        KeyCode::Enter => Some(Enter),
        KeyCode::Esc => Some(Escape),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SessionResult {
    Cancelled,
    Command(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RenderFrame {
    pub(crate) prefix: String,
    pub(crate) prefix_color: Option<u8>,
    pub(crate) input: String,
    /// UTF-8 byte offset at a character boundary.
    pub(crate) cursor: usize,
    pub(crate) input_color: Option<u8>,
    pub(crate) error: Option<String>,
    pub(crate) error_color: Option<u8>,
}

pub(crate) trait TerminalAdapter {
    fn next_event(&mut self) -> Result<TerminalEvent>;
    fn render(&mut self, frame: &RenderFrame) -> Result<()>;
    fn edit_external(&mut self, _text: &str) -> Result<String> {
        anyhow::bail!("external editor is not available")
    }
    fn complete(&mut self, text: &str, cursor: usize) -> Result<Option<String>> {
        super::input_tools::complete(text, cursor)
    }
    fn select_history(
        &mut self,
        history: &[String],
        style: &HistoryStyle,
    ) -> Result<Option<String>>;
    fn select_menu(&mut self, style: &MenuStyle) -> Result<Option<usize>>;
    fn select_provider(
        &mut self,
        providers: &[String],
        default: usize,
        style: &MenuStyle,
    ) -> Result<Option<usize>>;
    fn select_model(
        &mut self,
        models: &[String],
        default: usize,
        style: &MenuStyle,
    ) -> Result<Option<usize>>;
}

pub(crate) trait RequestHandler<S> {
    async fn suggest(&mut self, request: &str, state: &S) -> Result<String>;
    fn provider_options(&self) -> Vec<String>;
    fn model_options(&self, provider: &str) -> Vec<String>;
    fn current_model(&self, state: &S) -> Result<String>;
    fn select_model(&mut self, state: &mut S, selection: &str) -> Result<()>;
}

/// Runs a root request. A submitted prompt is stored before its provider request.
pub(crate) async fn run_session<S, T, H>(
    config: &SessionConfig,
    state: &mut S,
    history: &mut Vec<String>,
    terminal: &mut T,
    handler: &mut H,
) -> Result<SessionResult>
where
    T: TerminalAdapter,
    H: RequestHandler<S>,
{
    let mut request = InputBuffer::default();
    let mut draft = InputBuffer::default();
    let mut history_index = history.len();
    let mut error = None;
    loop {
        terminal.render(&frame(config, &request, error.take()))?;
        match terminal.next_event()? {
            TerminalEvent::Cancel => return Ok(SessionResult::Cancelled),
            TerminalEvent::Resize => {}
            TerminalEvent::SearchHistory => {
                if let Some(selected) = search_history(config, history, terminal, &request)? {
                    request.set(&selected);
                    history_index = history.len();
                }
            }
            TerminalEvent::EditExternal => match terminal.edit_external(&request.text) {
                Ok(text) => {
                    request.set(&text);
                    history_index = history.len();
                }
                Err(failure) => error = Some(failure.to_string()),
            },
            TerminalEvent::Complete => match terminal.complete(&request.text, request.cursor) {
                Ok(Some(suffix)) => {
                    request.edit(TerminalEvent::Paste(suffix));
                    history_index = history.len();
                }
                Ok(None) => {}
                Err(failure) => error = Some(failure.to_string()),
            },
            TerminalEvent::Character(character)
                if request.is_empty() && character == config.history_key =>
            {
                if let Some(selected) = terminal.select_history(history, &config.history_style)? {
                    request.set(&selected);
                    history_index = history.len();
                }
            }
            TerminalEvent::Character(character)
                if request.is_empty() && character == config.menu_key =>
            {
                loop {
                    let Some(index) = terminal.select_menu(&config.menu_style)? else {
                        break;
                    };

                    match index {
                        0 => {
                            let providers = handler.provider_options();
                            if providers.is_empty() {
                                error = Some("no providers are configured".to_owned());
                                break;
                            }
                            let current = handler.current_model(state)?;
                            let current_provider =
                                current.split_once('/').map(|(provider, _)| provider);
                            let provider = if providers.len() == 1 {
                                providers[0].clone()
                            } else {
                                let default = current_provider
                                    .and_then(|value| {
                                        providers.iter().position(|provider| provider == value)
                                    })
                                    .unwrap_or(0);
                                let Some(index) = terminal.select_provider(
                                    &providers,
                                    default,
                                    &config.menu_style,
                                )?
                                else {
                                    terminal.render(&frame(config, &request, None))?;
                                    continue;
                                };
                                providers[index].clone()
                            };
                            let models = handler.model_options(&provider);
                            if models.is_empty() {
                                error =
                                    Some("selected provider has no models configured".to_owned());
                                break;
                            }
                            let default = current_provider
                                .filter(|value| *value == provider)
                                .and_then(|_| current.split_once('/').map(|(_, model)| model))
                                .and_then(|value| models.iter().position(|model| model == value))
                                .unwrap_or(0);
                            if let Some(index) =
                                terminal.select_model(&models, default, &config.menu_style)?
                            {
                                handler.select_model(
                                    state,
                                    &format!("{provider}/{}", models[index]),
                                )?;
                                break;
                            }
                            terminal.render(&frame(config, &request, None))?;
                        }
                        _ => unreachable!("the session menu only contains configured commands"),
                    }
                }
            }
            TerminalEvent::Up if !history.is_empty() => {
                if history_index == history.len() {
                    draft = request.clone();
                }
                history_index = history_index.saturating_sub(1);
                request.set(&history[history_index]);
            }
            TerminalEvent::Down if history_index < history.len() => {
                history_index += 1;
                if let Some(entry) = history.get(history_index) {
                    request.set(entry);
                } else {
                    request.set(&draft.text);
                    request.cursor = draft.cursor;
                }
            }
            TerminalEvent::Up | TerminalEvent::Down => {}
            TerminalEvent::Escape if !request.is_empty() => {
                history_index = history.len();
                request.set("");
            }
            TerminalEvent::Escape => return Ok(SessionResult::Cancelled),
            TerminalEvent::Enter => {
                let request = request.text.trim();
                if request.is_empty() {
                    return Ok(SessionResult::Cancelled);
                }
                let request = request.to_owned();
                if !history.iter().any(|entry| entry == &request) {
                    history.push(request.clone());
                }
                history_index = history.len();
                match handler.suggest(&request, state).await {
                    Ok(command) => return Ok(SessionResult::Command(command)),
                    Err(request_error) => error = Some(request_error.to_string()),
                }
            }
            event => {
                if request.edit(event) {
                    history_index = history.len();
                }
            }
        }
    }
}

/// Search does not change the draft until Enter accepts a match.
fn search_history<T: TerminalAdapter>(
    config: &SessionConfig,
    history: &[String],
    terminal: &mut T,
    draft: &InputBuffer,
) -> Result<Option<String>> {
    let mut query = InputBuffer::default();
    let mut selected = history
        .iter()
        .rposition(|entry| entry.contains(&query.text));
    loop {
        let mut display = frame(config, &query, None);
        display.prefix = "search history: ".to_owned();
        display.error =
            Some(selected.map_or_else(|| "no match".to_owned(), |i| history[i].clone()));
        terminal.render(&display)?;
        match terminal.next_event()? {
            TerminalEvent::Enter => return Ok(selected.map(|i| history[i].clone())),
            TerminalEvent::Escape | TerminalEvent::Cancel => {
                terminal.render(&frame(config, draft, None))?;
                return Ok(None);
            }
            TerminalEvent::SearchHistory => {
                let end = selected.unwrap_or(history.len());
                selected = history[..end]
                    .iter()
                    .rposition(|entry| entry.contains(&query.text));
            }
            event => {
                if query.edit(event) {
                    selected = history
                        .iter()
                        .rposition(|entry| entry.contains(&query.text));
                }
            }
        }
    }
}

fn frame(config: &SessionConfig, request: &InputBuffer, error: Option<String>) -> RenderFrame {
    RenderFrame {
        prefix: config.prompt.clone(),
        prefix_color: config.prompt_color,
        input: request.text.clone(),
        cursor: request.cursor,
        input_color: config.input_color,
        error,
        error_color: config.error_color,
    }
}

/// An adapter for the controlling terminal.
pub(crate) struct CrosstermTerminal {
    output: std::fs::File,
    active: bool,
    origin: (u16, u16),
}

impl CrosstermTerminal {
    pub(crate) fn new() -> Result<Self> {
        setup_with_raw_mode(enable_raw_mode, disable_raw_mode, || {
            let mut output = std::fs::OpenOptions::new().write(true).open("/dev/tty")?;
            let origin = crossterm::cursor::position()?;
            if let Err(error) = setup_inline_terminal(&mut output) {
                let _ = execute!(output, DisableBracketedPaste);
                return Err(error);
            }
            Ok(Self {
                output,
                active: true,
                origin,
            })
        })
        .map_err(Into::into)
    }

    /// Clears this session's UI and returns the controlling terminal to normal mode.
    pub(crate) fn finish(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        restore_terminal(&mut self.output, disable_raw_mode).map_err(Into::into)
    }
}

impl Drop for CrosstermTerminal {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// Saves the shell cursor so the session can redraw only its own UI.
fn setup_inline_terminal(output: &mut impl Write) -> io::Result<()> {
    execute!(output, SavePosition, EnableBracketedPaste)
}

/// Restores the shell cursor and clears only the request UI after it.
fn clear_inline_terminal(output: &mut impl Write) -> io::Result<()> {
    execute!(output, RestorePosition, Clear(ClearType::FromCursorDown))
}

fn setup_with_raw_mode<T>(
    enable_raw: impl FnOnce() -> io::Result<()>,
    disable_raw: impl FnOnce() -> io::Result<()>,
    setup: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    enable_raw()?;
    let terminal = setup();
    if terminal.is_err() {
        let _ = disable_raw();
    }
    terminal
}

fn restore_terminal(
    output: &mut impl Write,
    disable_raw: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let cleared = clear_inline_terminal(output);
    let paste_disabled = execute!(output, DisableBracketedPaste);
    let raw_mode_disabled = disable_raw();
    cleared.and(paste_disabled).and(raw_mode_disabled)
}

/// Gives pickers their own row while keeping the request prompt visible.
fn prepare_menu_terminal(output: &mut impl Write) -> io::Result<()> {
    write!(output, "\r\n")?;
    output.flush()
}

/// Starts the history picker below the shell and AI prompts it follows.
fn prepare_history_terminal(output: &mut impl Write) -> io::Result<()> {
    prepare_menu_terminal(output)
}

fn render_nested_menu_header(
    output: &mut impl Write,
    prompt: &str,
    style: &MenuStyle,
) -> io::Result<()> {
    write_colored(output, prompt, style.prompt)?;
    write!(output, "\r\n")?;
    output.flush()
}

impl TerminalAdapter for CrosstermTerminal {
    fn next_event(&mut self) -> Result<TerminalEvent> {
        loop {
            match event::read()? {
                Event::Paste(text) => return Ok(TerminalEvent::Paste(text)),
                Event::Resize(_, _) => return Ok(TerminalEvent::Resize),
                Event::Key(key) => {
                    if let Some(event) = map_key(key) {
                        return Ok(event);
                    }
                }
                _ => {}
            }
        }
    }

    fn render(&mut self, frame: &RenderFrame) -> Result<()> {
        let size = crossterm::terminal::size()?;
        self.origin.0 = self.origin.0.min(size.0.saturating_sub(1));
        self.origin.1 = self.origin.1.min(size.1.saturating_sub(1));
        let layout = layout_frame(
            frame,
            usize::from(size.0.saturating_sub(self.origin.0).max(1)),
        );
        let rows = layout
            .cells
            .last()
            .map_or(1, |cell| cell.0 + 1)
            .max(layout.cursor.0 + 1);
        let needed = rows.min(usize::from(size.1.max(1))) as u16;
        let scroll = (self.origin.1 + needed).saturating_sub(size.1.max(1));
        if scroll > 0 {
            execute!(self.output, ScrollUp(scroll))?;
            self.origin.1 -= scroll;
        }
        execute!(
            self.output,
            MoveTo(self.origin.0, self.origin.1),
            SavePosition
        )?;
        render_frame(&mut self.output, frame, self.origin, size)?;
        Ok(())
    }

    fn edit_external(&mut self, text: &str) -> Result<String> {
        execute!(self.output, DisableBracketedPaste)?;
        disable_raw_mode()?;
        let result = super::input_tools::edit_external(text);
        enable_raw_mode()?;
        execute!(self.output, EnableBracketedPaste)?;
        result
    }

    fn select_history(
        &mut self,
        history: &[String],
        style: &HistoryStyle,
    ) -> Result<Option<String>> {
        execute!(self.output, DisableBracketedPaste)?;
        disable_raw_mode()?;
        let selected = (|| {
            prepare_history_terminal(&mut self.output)?;
            run_history_picker(history, style)
        })();
        enable_raw_mode()?;
        execute!(self.output, EnableBracketedPaste)?;
        selected
    }

    fn select_menu(&mut self, style: &MenuStyle) -> Result<Option<usize>> {
        self.select_items(&["model".to_owned()], 0, style)
    }

    fn select_provider(
        &mut self,
        providers: &[String],
        default: usize,
        style: &MenuStyle,
    ) -> Result<Option<usize>> {
        self.select_nested_items("provider:", providers, default, style)
    }

    fn select_model(
        &mut self,
        models: &[String],
        default: usize,
        style: &MenuStyle,
    ) -> Result<Option<usize>> {
        self.select_nested_items("model:", models, default, style)
    }
}

impl CrosstermTerminal {
    fn select_items(
        &mut self,
        items: &[String],
        default: usize,
        style: &MenuStyle,
    ) -> Result<Option<usize>> {
        self.select_with_theme(items, default, style, "")
    }

    fn select_nested_items(
        &mut self,
        prompt: &str,
        items: &[String],
        default: usize,
        style: &MenuStyle,
    ) -> Result<Option<usize>> {
        execute!(self.output, DisableBracketedPaste)?;
        disable_raw_mode()?;
        let theme = SessionMenuTheme {
            style: style.clone(),
            item_indent: "  ",
        };
        let selected = (|| {
            render_nested_menu_header(&mut self.output, prompt, style)?;
            Select::with_theme(&theme)
                .items(items)
                .default(default)
                .report(false)
                .interact_opt()
        })();
        enable_raw_mode()?;
        execute!(self.output, EnableBracketedPaste)?;
        Ok(selected?)
    }

    fn select_with_theme(
        &mut self,
        items: &[String],
        default: usize,
        style: &MenuStyle,
        item_indent: &'static str,
    ) -> Result<Option<usize>> {
        execute!(self.output, DisableBracketedPaste)?;
        disable_raw_mode()?;
        let theme = SessionMenuTheme {
            style: style.clone(),
            item_indent,
        };
        let selected = (|| {
            prepare_menu_terminal(&mut self.output)?;
            Select::with_theme(&theme)
                .items(items)
                .default(default)
                .report(false)
                .interact_opt()
        })();
        enable_raw_mode()?;
        execute!(self.output, EnableBracketedPaste)?;
        Ok(selected?)
    }
}

struct SessionMenuTheme {
    style: MenuStyle,
    item_indent: &'static str,
}

impl Theme for SessionMenuTheme {
    fn format_select_prompt(&self, f: &mut dyn std::fmt::Write, prompt: &str) -> std::fmt::Result {
        format_color(f, prompt, self.style.prompt)
    }

    fn format_select_prompt_item(
        &self,
        f: &mut dyn std::fmt::Write,
        text: &str,
        active: bool,
    ) -> std::fmt::Result {
        format_color(
            f,
            &format!(
                "{}{} {text}",
                self.item_indent,
                if active { '›' } else { ' ' }
            ),
            if active {
                self.style.active
            } else {
                self.style.item
            },
        )
    }
}

#[derive(Debug)]
struct InputLayout {
    cells: Vec<(usize, usize, String, Option<u8>)>,
    cursor: (usize, usize),
}

fn layout_frame(frame: &RenderFrame, width: usize) -> InputLayout {
    let width = width.max(1);
    let mut cells = Vec::new();
    let (mut row, mut column) = (0, 0);
    let mut cursor = None;
    for (text, color, input) in [
        (frame.prefix.as_str(), frame.prefix_color, false),
        (frame.input.as_str(), frame.input_color, true),
    ] {
        for (offset, grapheme) in text.grapheme_indices(true) {
            let c: String = grapheme
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            let mut size = c.width();
            let c = if size > width {
                size = 1;
                "\u{fffd}".to_owned()
            } else {
                c
            };
            if column + size > width {
                row += 1;
                column = 0;
            }
            if input && (offset..offset + grapheme.len()).contains(&frame.cursor) {
                cursor = Some((row, column));
            }
            cells.push((row, column, c, color));
            column += size;
            // Explicit positioning avoids terminal autowrap and bottom-row scrolling.
            if column == width {
                row += 1;
                column = 0;
            }
        }
    }
    let cursor = cursor.unwrap_or((row, column));
    if let Some(error) = &frame.error {
        row += 1;
        column = 0;
        for grapheme in error.graphemes(true) {
            let c: String = grapheme
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            let mut size = c.width();
            let c = if size > width {
                size = 1;
                "\u{fffd}".to_owned()
            } else {
                c
            };
            if column + size > width {
                row += 1;
                column = 0;
            }
            cells.push((row, column, c, frame.error_color));
            column += size;
            if column == width {
                row += 1;
                column = 0;
            }
        }
    }
    InputLayout { cells, cursor }
}

fn render_frame(
    output: &mut impl Write,
    frame: &RenderFrame,
    origin: (u16, u16),
    size: (u16, u16),
) -> io::Result<()> {
    let x = origin.0.min(size.0.saturating_sub(1));
    let y = origin.1.min(size.1.saturating_sub(1));
    let width = usize::from(size.0.saturating_sub(x).max(1));
    let height = usize::from(size.1.saturating_sub(y).max(1));
    let layout = layout_frame(frame, width);
    let first = layout.cursor.0.saturating_sub(height - 1);
    execute!(output, MoveTo(x, y), Clear(ClearType::FromCursorDown))?;
    for (row, column, c, color) in layout.cells {
        if row >= first && row - first < height {
            execute!(output, MoveTo(x + column as u16, y + (row - first) as u16))?;
            write_colored(output, &c, color)?;
        }
    }
    execute!(
        output,
        MoveTo(
            x + layout.cursor.1 as u16,
            y + (layout.cursor.0 - first) as u16
        )
    )?;
    output.flush()
}

fn format_color(f: &mut dyn std::fmt::Write, text: &str, color: Option<u8>) -> std::fmt::Result {
    match color {
        Some(color) => write!(f, "\x1b[38;5;{color}m{text}\x1b[0m"),
        None => write!(f, "{text}"),
    }
}

fn run_history_picker(history: &[String], style: &HistoryStyle) -> Result<Option<String>> {
    super::history_picker::pick(
        history,
        [
            style.prompt,
            style.input,
            style.item,
            style.matched,
            style.active,
            style.active_match,
        ],
    )
}

fn write_colored(output: &mut dyn Write, text: &str, color: Option<u8>) -> io::Result<()> {
    if let Some(color) = color {
        write!(output, "\x1b[38;5;{color}m{text}\x1b[0m")
    } else {
        write!(output, "{text}")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use anyhow::bail;

    use super::*;

    struct ScriptedTerminal {
        events: VecDeque<TerminalEvent>,
        history_selections: VecDeque<Option<String>>,
        menu_selections: VecDeque<Option<usize>>,
        frames: Vec<RenderFrame>,
        history_styles: Vec<HistoryStyle>,
        menu_styles: Vec<MenuStyle>,
        editor_results: VecDeque<Result<String>>,
        completions: VecDeque<Result<Option<String>>>,
    }

    impl ScriptedTerminal {
        fn new(events: impl IntoIterator<Item = TerminalEvent>) -> Self {
            Self {
                events: events.into_iter().collect(),
                history_selections: VecDeque::new(),
                menu_selections: VecDeque::new(),
                frames: Vec::new(),
                history_styles: Vec::new(),
                menu_styles: Vec::new(),
                editor_results: VecDeque::new(),
                completions: VecDeque::new(),
            }
        }

        fn with_history_selection(mut self, selection: Option<&str>) -> Self {
            self.history_selections
                .push_back(selection.map(str::to_owned));
            self
        }

        fn with_menu_selections(
            mut self,
            selections: impl IntoIterator<Item = Option<usize>>,
        ) -> Self {
            self.menu_selections.extend(selections);
            self
        }
    }

    impl TerminalAdapter for ScriptedTerminal {
        fn next_event(&mut self) -> Result<TerminalEvent> {
            self.events
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no event"))
        }

        fn render(&mut self, frame: &RenderFrame) -> Result<()> {
            self.frames.push(frame.clone());
            Ok(())
        }

        fn edit_external(&mut self, _text: &str) -> Result<String> {
            self.editor_results.pop_front().unwrap()
        }

        fn complete(&mut self, _text: &str, _cursor: usize) -> Result<Option<String>> {
            self.completions.pop_front().unwrap()
        }

        fn select_history(
            &mut self,
            _history: &[String],
            style: &HistoryStyle,
        ) -> Result<Option<String>> {
            self.history_styles.push(style.clone());
            Ok(self.history_selections.pop_front().flatten())
        }

        fn select_menu(&mut self, style: &MenuStyle) -> Result<Option<usize>> {
            self.menu_styles.push(style.clone());
            Ok(self.menu_selections.pop_front().flatten())
        }

        fn select_provider(
            &mut self,
            _providers: &[String],
            _default: usize,
            style: &MenuStyle,
        ) -> Result<Option<usize>> {
            self.menu_styles.push(style.clone());
            Ok(self.menu_selections.pop_front().flatten())
        }

        fn select_model(
            &mut self,
            _models: &[String],
            _default: usize,
            style: &MenuStyle,
        ) -> Result<Option<usize>> {
            self.menu_styles.push(style.clone());
            Ok(self.menu_selections.pop_front().flatten())
        }
    }

    struct StubHandler(Result<String>);

    impl StubHandler {
        fn success(command: &str) -> Self {
            Self(Ok(command.to_owned()))
        }
    }

    impl RequestHandler<()> for StubHandler {
        async fn suggest(&mut self, _request: &str, _state: &()) -> Result<String> {
            match &self.0 {
                Ok(command) => Ok(command.clone()),
                Err(_) => bail!("provider failed"),
            }
        }

        fn provider_options(&self) -> Vec<String> {
            vec!["test".to_owned()]
        }

        fn model_options(&self, _provider: &str) -> Vec<String> {
            vec!["model".to_owned()]
        }

        fn current_model(&self, _state: &()) -> Result<String> {
            Ok("test/model".to_owned())
        }

        fn select_model(&mut self, _state: &mut (), _selection: &str) -> Result<()> {
            Ok(())
        }
    }

    struct ModelHandler;

    impl RequestHandler<String> for ModelHandler {
        async fn suggest(&mut self, _request: &str, _state: &String) -> Result<String> {
            Ok("unused".to_owned())
        }

        fn provider_options(&self) -> Vec<String> {
            vec!["other".to_owned(), "test".to_owned()]
        }

        fn model_options(&self, provider: &str) -> Vec<String> {
            match provider {
                "other" => vec!["third".to_owned()],
                "test" => vec!["first".to_owned(), "second".to_owned()],
                _ => Vec::new(),
            }
        }

        fn current_model(&self, state: &String) -> Result<String> {
            Ok(state.clone())
        }

        fn select_model(&mut self, state: &mut String, selection: &str) -> Result<()> {
            state.clone_from(&selection.to_owned());
            Ok(())
        }
    }

    fn config() -> SessionConfig {
        SessionConfig {
            prompt: "AI› ".to_owned(),
            prompt_color: Some(250),
            input_color: Some(252),
            error_color: Some(203),
            history_key: '!',
            menu_key: '/',
            history_style: HistoryStyle {
                prompt: Some(250),
                input: Some(252),
                item: Some(252),
                matched: Some(110),
                active: Some(110),
                active_match: Some(110),
            },
            menu_style: MenuStyle {
                prompt: Some(250),
                item: Some(252),
                active: Some(110),
            },
        }
    }

    #[test]
    fn edits_at_utf8_boundaries_and_stops_at_line_edges() {
        use TerminalEvent::*;
        let mut input = InputBuffer::default();
        input.edit(Backspace);
        input.edit(Delete);
        input.edit(Left);
        input.edit(Right);
        assert_eq!(input.cursor, 0);
        input.insert("a界é");
        input.edit(Left);
        assert_eq!(input.cursor, 4);
        input.edit(Character('!'));
        assert_eq!(input.text, "a界!é");
        input.edit(Delete);
        assert_eq!(input.text, "a界!");
        input.edit(Backspace);
        input.edit(Backspace);
        assert_eq!(input.text, "a");
        input.edit(Home);
        input.edit(Character('é'));
        input.edit(End);
        input.edit(Right);
        assert_eq!((input.text.as_str(), input.cursor), ("éa", 3));
    }

    #[test]
    fn moves_by_words_and_restores_killed_text_at_the_cursor() {
        use TerminalEvent::*;
        let mut input = InputBuffer::default();
        input.insert("one  世界 three  ");
        input.edit(WordLeft);
        assert_eq!(&input.text[input.cursor..], "three  ");
        input.edit(WordLeft);
        assert_eq!(&input.text[input.cursor..], "世界 three  ");
        input.edit(WordRight);
        assert_eq!(&input.text[input.cursor..], " three  ");
        input.edit(KillWord);
        assert_eq!(input.text, "one   three  ");
        input.edit(Yank);
        assert_eq!(input.text, "one  世界 three  ");
        input.edit(KillStart);
        assert_eq!(input.text, " three  ");
        input.edit(KillStart); // Empty kills retain the last deletion.
        input.edit(Yank);
        input.edit(KillEnd);
        assert_eq!(input.text, "one  世界");
        input.edit(Home);
        input.edit(Yank);
        assert_eq!(input.text, " three  one  世界");
    }

    #[test]
    fn maps_supported_shortcuts_without_binding_clipboard_controls() {
        use KeyCode::*;
        use KeyModifiers as M;
        let cases = [
            (Left, M::NONE, TerminalEvent::Left),
            (Right, M::NONE, TerminalEvent::Right),
            (Home, M::NONE, TerminalEvent::Home),
            (End, M::NONE, TerminalEvent::End),
            (Backspace, M::NONE, TerminalEvent::Backspace),
            (Delete, M::NONE, TerminalEvent::Delete),
            (Char('a'), M::CONTROL, TerminalEvent::Home),
            (Char('e'), M::CONTROL, TerminalEvent::End),
            (Char('w'), M::CONTROL, TerminalEvent::KillWord),
            (Char('u'), M::CONTROL, TerminalEvent::KillStart),
            (Char('k'), M::CONTROL, TerminalEvent::KillEnd),
            (Char('y'), M::CONTROL, TerminalEvent::Yank),
            (Left, M::CONTROL, TerminalEvent::WordLeft),
            (Right, M::CONTROL, TerminalEvent::WordRight),
            (Left, M::ALT, TerminalEvent::WordLeft),
            (Right, M::ALT, TerminalEvent::WordRight),
            (Char('b'), M::ALT, TerminalEvent::WordLeft),
            (Char('f'), M::ALT, TerminalEvent::WordRight),
            (Char('A'), M::SHIFT, TerminalEvent::Character('A')),
        ];
        for (code, modifiers, expected) in cases {
            assert_eq!(map_key(KeyEvent::new(code, modifiers)), Some(expected));
        }
        for (code, modifiers) in [
            (Char('c'), M::CONTROL | M::SHIFT),
            (Char('v'), M::CONTROL | M::SHIFT),
            (Char('c'), M::SUPER),
            (Char('v'), M::SUPER),
            (Char('v'), M::CONTROL),
            (Insert, M::SHIFT),
            (Left, M::SHIFT),
        ] {
            assert_eq!(map_key(KeyEvent::new(code, modifiers)), None);
        }
        assert_eq!(
            map_key(KeyEvent::new_with_kind(
                Left,
                M::NONE,
                KeyEventKind::Release
            )),
            None
        );
        assert_eq!(
            map_key(KeyEvent::new_with_kind(Left, M::NONE, KeyEventKind::Repeat)),
            Some(TerminalEvent::Left)
        );
    }

    #[test]
    fn undo_redo_restore_text_cursor_and_invalidate_only_on_changes() {
        use TerminalEvent::*;
        let mut input = InputBuffer::default();
        input.edit(Paste("a界é".into()));
        input.edit(Left);
        input.edit(Backspace);
        assert_eq!((&*input.text, input.cursor), ("aé", 1));
        input.edit(Undo);
        assert_eq!((&*input.text, input.cursor), ("a界é", 4));
        input.edit(Home);
        input.edit(Backspace); // No change must not clear redo.
        input.edit(Redo);
        assert_eq!((&*input.text, input.cursor), ("aé", 1));
        input.edit(Undo);
        input.edit(Character('!'));
        assert!(!input.edit(Redo));
        input.set("history");
        input.edit(Undo);
        assert_eq!(input.text, "!a界é");
    }

    #[test]
    fn wraps_wide_graphemes_exact_edges_and_combining_text() {
        use TerminalEvent::*;
        let mut input = InputBuffer::default();
        input.set("a界e\u{301}👩‍💻z");
        let mut display = frame(&config(), &input, None);
        display.prefix.clear();
        assert_eq!(layout_frame(&display, 4).cursor, (1, 3));
        display.cursor = 1;
        assert_eq!(layout_frame(&display, 2).cursor, (1, 0));
        display.input = "abcd".into();
        display.cursor = 4;
        assert_eq!(layout_frame(&display, 4).cursor, (1, 0));
        input.edit(Left);
        input.edit(Backspace);
        assert_eq!(input.text, "a界e\u{301}z");
        input.edit(Backspace);
        assert_eq!(input.text, "a界z");
    }

    #[test]
    fn viewport_keeps_cursor_visible_after_resize_without_scrolling_output() {
        let mut input = InputBuffer::default();
        input.set(&"界".repeat(50));
        let display = frame(&config(), &input, None);
        for size in [(8, 3), (1, 1), (0, 0), (20, 5)] {
            let mut output = Vec::new();
            render_frame(&mut output, &display, (4, 2), size).unwrap();
            assert!(!output.contains(&b'\n'));
            assert!(!output.windows(2).any(|bytes| bytes == b"\x1bD"));
            let text = String::from_utf8(output).unwrap();
            let last = text.rsplit("\x1b[").next().unwrap().trim_end_matches('H');
            let (row, col) = last.split_once(';').unwrap();
            assert!(row.parse::<u16>().unwrap() <= size.1.max(1));
            assert!(col.parse::<u16>().unwrap() <= size.0.max(1));
        }
    }

    #[test]
    fn input_shortcuts_do_not_claim_clipboard_or_job_control_keys() {
        use KeyModifiers as M;
        for (code, modifiers, expected) in [
            (KeyCode::Char('_'), M::CONTROL, TerminalEvent::Undo),
            (KeyCode::Char('7'), M::CONTROL, TerminalEvent::Undo),
            (KeyCode::Char('/'), M::CONTROL, TerminalEvent::Undo),
            (KeyCode::Char('u'), M::ALT, TerminalEvent::Undo),
            (KeyCode::Char('r'), M::ALT, TerminalEvent::Redo),
            (KeyCode::Char('r'), M::CONTROL, TerminalEvent::SearchHistory),
            (KeyCode::Char('e'), M::ALT, TerminalEvent::EditExternal),
            (KeyCode::Char('c'), M::CONTROL, TerminalEvent::Cancel),
            (KeyCode::Tab, M::NONE, TerminalEvent::Complete),
        ] {
            assert_eq!(map_key(KeyEvent::new(code, modifiers)), Some(expected));
        }
        for (code, modifiers) in [
            (KeyCode::Char('z'), M::CONTROL),
            (KeyCode::Char('c'), M::CONTROL | M::SHIFT),
            (KeyCode::Char('v'), M::CONTROL | M::SHIFT),
            (KeyCode::Char('v'), M::SUPER),
        ] {
            assert_eq!(map_key(KeyEvent::new(code, modifiers)), None);
        }
    }

    #[tokio::test]
    async fn reverse_search_cycles_accepts_without_submitting_and_can_be_undone() {
        use TerminalEvent::*;
        let mut terminal = ScriptedTerminal::new([
            Paste("draft".into()),
            SearchHistory,
            Paste("git".into()),
            SearchHistory,
            Enter,
            Undo,
            Redo,
            Enter,
        ]);
        let mut history = vec!["git log".into(), "ls".into(), "git status".into()];
        let mut handler = StubHandler::success("unused");
        run_session(
            &config(),
            &mut (),
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();
        assert_eq!(terminal.frames.last().unwrap().input, "git log");
        assert!(
            terminal
                .frames
                .iter()
                .any(|f| f.input == "draft" && f.prefix == config().prompt)
        );
        assert_eq!(history.len(), 3);
    }

    #[tokio::test]
    async fn reverse_search_cancel_and_missing_match_preserve_draft_cursor() {
        use TerminalEvent::*;
        for exit in [Escape, Enter] {
            let mut terminal = ScriptedTerminal::new([
                Paste("draft".into()),
                Left,
                SearchHistory,
                Paste("missing".into()),
                exit,
                Character('!'),
                Enter,
            ]);
            let mut history = vec!["git log".into()];
            let mut handler = StubHandler::success("unused");
            run_session(
                &config(),
                &mut (),
                &mut history,
                &mut terminal,
                &mut handler,
            )
            .await
            .unwrap();
            assert_eq!(history.last().unwrap(), "draf!t");
        }
        let mut terminal = ScriptedTerminal::new([SearchHistory, SearchHistory, Enter]);
        assert_eq!(
            search_history(&config(), &[], &mut terminal, &InputBuffer::default()).unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn editor_and_completion_are_undoable_and_errors_preserve_the_prompt() {
        use TerminalEvent::*;
        let mut terminal = ScriptedTerminal::new([
            Paste("open fol later".into()),
            Home,
            Right,
            Right,
            Right,
            Right,
            Right,
            Right,
            Right,
            Right,
            Complete,
            Undo,
            Redo,
            EditExternal,
            Undo,
            Redo,
            EditExternal,
            Enter,
        ]);
        terminal.completions.push_back(Ok(Some("der/".into())));
        terminal
            .editor_results
            .push_back(Ok("edited\nrequest\x1b".into()));
        terminal
            .editor_results
            .push_back(Err(anyhow::anyhow!("editor failed")));
        let mut history = Vec::new();
        let mut handler = StubHandler::success("unused");
        run_session(
            &config(),
            &mut (),
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();
        assert!(
            terminal
                .frames
                .iter()
                .any(|f| f.input == "open folder/ later")
        );
        assert_eq!(history, ["edited request"]);
        assert_eq!(
            terminal.frames.last().unwrap().error.as_deref(),
            Some("editor failed")
        );
    }

    #[test]
    fn renders_cursor_using_display_width_not_byte_length() {
        let mut input = InputBuffer::default();
        input.insert("界éx");
        input.edit(TerminalEvent::Left);
        let frame = frame(&config(), &input, Some("error".to_owned()));
        assert_eq!(layout_frame(&frame, 80).cursor, (0, 7));
        let mut output = Vec::new();
        render_frame(&mut output, &frame, (0, 0), (80, 24)).unwrap();
        assert!(output.ends_with(b"\x1b[1;8H"));
    }

    #[tokio::test]
    async fn paste_is_sanitized_without_submission_or_menu_actions() {
        let mut terminal = ScriptedTerminal::new([
            TerminalEvent::Paste("/one\r\ntwo\rthree\tfour\x1b\x00".to_owned()),
            TerminalEvent::Home,
            TerminalEvent::Character('!'),
            TerminalEvent::Enter,
        ]);
        let mut history = Vec::new();
        let mut handler = StubHandler::success("unused");
        run_session(
            &config(),
            &mut (),
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();
        assert_eq!(terminal.frames[1].input, "/one two three four");
        assert_eq!(history, ["!/one two three four"]);
        assert!(terminal.menu_styles.is_empty());
        assert!(terminal.history_styles.is_empty());
    }

    #[tokio::test]
    async fn history_restores_the_draft_and_cursor_and_allows_edits() {
        use TerminalEvent::*;
        let mut terminal = ScriptedTerminal::new([
            Paste("draft".to_owned()),
            Left,
            Up,
            Up,
            Up,
            Down,
            Down,
            Character('!'),
            Up,
            Left,
            Backspace,
            Enter,
        ]);
        let mut history = vec!["first".to_owned(), "last".to_owned()];
        let mut handler = StubHandler::success("unused");
        run_session(
            &config(),
            &mut (),
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();
        assert_eq!(terminal.frames[5].input, "first");
        assert_eq!(terminal.frames[7].input, "draft");
        assert_eq!(terminal.frames[7].cursor, 4);
        assert_eq!(terminal.frames[8].input, "draf!t");
        assert_eq!(history.last().unwrap(), "lat");
    }

    #[test]
    fn request_session_does_not_switch_to_the_alternate_screen() {
        let mut output = Vec::new();

        setup_inline_terminal(&mut output).unwrap();

        assert_eq!(output, b"\x1b7\x1b[?2004h");
    }

    #[test]
    fn clearing_the_inline_session_removes_the_request_line() {
        let mut output = Vec::new();

        clear_inline_terminal(&mut output).unwrap();

        assert_eq!(output, b"\x1b8\x1b[J");
    }

    #[test]
    fn terminal_setup_restores_normal_mode_when_setup_fails() {
        let mut raw_mode_disabled = false;

        assert!(
            setup_with_raw_mode(
                || Ok(()),
                || {
                    raw_mode_disabled = true;
                    Ok(())
                },
                || Err::<(), _>(io::Error::other("setup failed")),
            )
            .is_err()
        );
        assert!(raw_mode_disabled);
    }

    #[test]
    fn terminal_cleanup_disables_raw_mode_after_clearing_the_inline_session() {
        let mut output = Vec::new();
        let mut raw_mode_disabled = false;

        restore_terminal(&mut output, || {
            raw_mode_disabled = true;
            Ok(())
        })
        .unwrap();

        assert_eq!(output, b"\x1b8\x1b[J\x1b[?2004l");
        assert!(raw_mode_disabled);
    }

    #[test]
    fn terminal_cleanup_disables_raw_mode_when_inline_cleanup_fails() {
        struct FailingWriter;

        impl Write for FailingWriter {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("write failed"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut output = FailingWriter;
        let mut raw_mode_disabled = false;

        assert!(
            restore_terminal(&mut output, || {
                raw_mode_disabled = true;
                Ok(())
            })
            .is_err()
        );
        assert!(raw_mode_disabled);
    }

    #[test]
    fn menu_picker_starts_below_the_visible_ai_prompt() {
        let mut output = Vec::new();

        prepare_menu_terminal(&mut output).unwrap();

        assert_eq!(output, b"\r\n");
    }

    #[test]
    fn history_picker_starts_below_the_visible_shell_and_ai_prompts() {
        let mut output = Vec::new();

        prepare_history_terminal(&mut output).unwrap();

        assert_eq!(output, b"\r\n");
    }

    #[test]
    fn nested_model_picker_renders_an_indented_model_menu() {
        let mut output = Vec::new();

        render_nested_menu_header(&mut output, "model:", &MenuStyle::default()).unwrap();

        assert_eq!(output, b"model:\r\n");
    }

    #[tokio::test]
    async fn escape_clears_a_typed_request_before_cancelling_the_session() {
        let mut terminal = ScriptedTerminal::new([
            TerminalEvent::Character('l'),
            TerminalEvent::Escape,
            TerminalEvent::Escape,
        ]);
        let mut history = Vec::new();
        let mut handler = StubHandler::success("ls");

        let result = run_session(
            &config(),
            &mut (),
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();

        assert_eq!(result, SessionResult::Cancelled);
        assert_eq!(terminal.frames[0].input, "");
        assert_eq!(terminal.frames[0].prefix, "AI› ");
        assert_eq!(terminal.frames[1].input, "l");
        assert_eq!(terminal.frames[2].input, "");
    }

    #[tokio::test]
    async fn records_a_submitted_request_and_returns_its_suggested_command() {
        let mut terminal = ScriptedTerminal::new([
            TerminalEvent::Character('l'),
            TerminalEvent::Character('s'),
            TerminalEvent::Enter,
        ]);
        let mut history = vec!["pwd".to_owned()];
        let mut handler = StubHandler::success("ls -la");

        let result = run_session(
            &config(),
            &mut (),
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();

        assert_eq!(result, SessionResult::Command("ls -la".to_owned()));
        assert_eq!(history, ["pwd", "ls"]);
    }

    #[tokio::test]
    async fn recalls_history_and_returns_to_an_empty_request() {
        let mut terminal = ScriptedTerminal::new([
            TerminalEvent::Up,
            TerminalEvent::Down,
            TerminalEvent::Escape,
        ]);
        let mut history = vec!["pwd".to_owned(), "git status".to_owned()];
        let mut handler = StubHandler::success("unused");

        let result = run_session(
            &config(),
            &mut (),
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();

        assert_eq!(result, SessionResult::Cancelled);
        assert_eq!(terminal.frames[1].input, "git status");
        assert_eq!(terminal.frames[2].input, "");
    }

    #[tokio::test]
    async fn escape_leaves_history_navigation_before_cancelling_the_session() {
        let mut terminal = ScriptedTerminal::new([
            TerminalEvent::Up,
            TerminalEvent::Escape,
            TerminalEvent::Escape,
        ]);
        let mut history = vec!["pwd".to_owned(), "git status".to_owned()];
        let mut handler = StubHandler::success("unused");

        let result = run_session(
            &config(),
            &mut (),
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();

        assert_eq!(result, SessionResult::Cancelled);
        assert_eq!(terminal.frames[1].input, "git status");
        assert_eq!(terminal.frames[2].input, "");
    }

    #[tokio::test]
    async fn escape_clears_recalled_history_before_cancelling_the_session() {
        let mut terminal = ScriptedTerminal::new([
            TerminalEvent::Character('!'),
            TerminalEvent::Character('!'),
            TerminalEvent::Escape,
            TerminalEvent::Escape,
        ])
        .with_history_selection(Some("git status"));
        let mut history = vec!["git status".to_owned()];
        let mut handler = StubHandler::success("unused");

        let result = run_session(
            &config(),
            &mut (),
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();

        assert_eq!(result, SessionResult::Cancelled);
        assert_eq!(terminal.frames[1].input, "git status");
        assert_eq!(terminal.frames[2].input, "git status!");
        assert_eq!(terminal.frames[3].input, "");
        assert_eq!(terminal.history_styles, [config().history_style]);
    }

    #[tokio::test]
    async fn selects_a_model_from_the_configured_menu_key() {
        let mut config = config();
        config.menu_key = '@';
        let mut terminal =
            ScriptedTerminal::new([TerminalEvent::Character('@'), TerminalEvent::Escape])
                .with_menu_selections([Some(0), Some(1), Some(1)]);
        let mut history = Vec::new();
        let mut state = "test/first".to_owned();
        let mut handler = ModelHandler;

        let result = run_session(
            &config,
            &mut state,
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();

        assert_eq!(result, SessionResult::Cancelled);
        assert_eq!(state, "test/second");
        assert_eq!(
            terminal.menu_styles,
            [
                config.menu_style.clone(),
                config.menu_style.clone(),
                config.menu_style,
            ]
        );
    }

    #[tokio::test]
    async fn escape_from_a_nested_menu_returns_to_its_parent_menu() {
        let mut terminal =
            ScriptedTerminal::new([TerminalEvent::Character('/'), TerminalEvent::Escape])
                .with_menu_selections([Some(0), Some(0), None, None]);
        let mut history = Vec::new();
        let mut state = "test/first".to_owned();
        let mut handler = ModelHandler;

        let result = run_session(
            &config(),
            &mut state,
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();

        assert_eq!(result, SessionResult::Cancelled);
        assert_eq!(
            terminal.menu_styles,
            [
                config().menu_style,
                config().menu_style,
                config().menu_style,
                config().menu_style
            ]
        );
        assert_eq!(terminal.frames.len(), 3);
        assert_eq!(state, "test/first");
    }

    #[tokio::test]
    async fn renders_a_themed_error_and_keeps_the_request_editable() {
        let mut terminal = ScriptedTerminal::new([
            TerminalEvent::Character('l'),
            TerminalEvent::Enter,
            TerminalEvent::Escape,
            TerminalEvent::Escape,
        ]);
        let mut history = Vec::new();
        let mut handler = StubHandler(Err(anyhow::anyhow!("provider failed")));

        let result = run_session(
            &config(),
            &mut (),
            &mut history,
            &mut terminal,
            &mut handler,
        )
        .await
        .unwrap();

        assert_eq!(result, SessionResult::Cancelled);
        let error_frame = &terminal.frames[2];
        assert_eq!(error_frame.input, "l");
        assert_eq!(error_frame.error.as_deref(), Some("provider failed"));
        assert_eq!(error_frame.error_color, Some(203));
    }
}
