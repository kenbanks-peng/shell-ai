//! Rust-owned terminal request session.

use std::io::{self, Write};

use anyhow::Result;
use crossterm::{
    cursor::{MoveRight, RestorePosition, SavePosition},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode},
};
use dialoguer::{Select, theme::Theme};
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
}

impl InputBuffer {
    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn set(&mut self, text: &str) {
        self.text.clear();
        self.cursor = 0;
        self.insert(text);
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
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next(&self) -> usize {
        self.cursor
            + self.text[self.cursor..]
                .chars()
                .next()
                .map_or(0, char::len_utf8)
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
            _ => None,
        };
    }
    if modifiers == KeyModifiers::ALT {
        return match key.code {
            KeyCode::Left | KeyCode::Char('b') => Some(WordLeft),
            KeyCode::Right | KeyCode::Char('f') => Some(WordRight),
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
                    request.text.clone_from(&draft.text);
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
}

impl CrosstermTerminal {
    pub(crate) fn new() -> Result<Self> {
        setup_with_raw_mode(enable_raw_mode, disable_raw_mode, || {
            let mut output = std::fs::OpenOptions::new().write(true).open("/dev/tty")?;
            if let Err(error) = setup_inline_terminal(&mut output) {
                let _ = execute!(output, DisableBracketedPaste);
                return Err(error);
            }
            Ok(Self {
                output,
                active: true,
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
        render_frame(&mut self.output, frame)?;
        Ok(())
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

fn render_frame(output: &mut impl Write, frame: &RenderFrame) -> io::Result<()> {
    clear_inline_terminal(output)?;
    write_colored(output, &frame.prefix, frame.prefix_color)?;
    if !frame.input.is_empty() {
        write_colored(output, &frame.input, frame.input_color)?;
    }
    if let Some(error) = &frame.error {
        write!(output, "\r\n")?;
        write_colored(output, error, frame.error_color)?;
    }
    execute!(output, RestorePosition, MoveRight(input_column(frame)))?;
    output.flush()
}

fn input_column(frame: &RenderFrame) -> u16 {
    let width = frame.prefix.width() + frame.input[..frame.cursor].width();
    width.min(u16::MAX as usize) as u16
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
    fn renders_cursor_using_display_width_not_byte_length() {
        let mut input = InputBuffer::default();
        input.insert("界éx");
        input.edit(TerminalEvent::Left);
        let frame = frame(&config(), &input, Some("error".to_owned()));
        assert_eq!(input_column(&frame), 7);
        let mut output = Vec::new();
        render_frame(&mut output, &frame).unwrap();
        assert!(output.ends_with(b"\x1b8\x1b[7C"));
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
