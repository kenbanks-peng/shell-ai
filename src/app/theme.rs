//! Terminal theme loading and dialoguer rendering.

use std::{fmt, fs, path::Path};

use anyhow::{Context, Result, bail};
use dialoguer::theme::Theme;
use serde::Deserialize;

use super::config::Config;

#[derive(Debug, Default, Deserialize)]
struct ThemeFile {
    #[serde(default)]
    colors: Colors,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Colors {
    pub(crate) ai_prompt: Option<u8>,
    pub(crate) input: Option<u8>,
    pub(crate) menu_prompt: Option<u8>,
    pub(crate) menu_item: Option<u8>,
    pub(crate) menu_active: Option<u8>,
    pub(crate) history_prompt: Option<u8>,
    pub(crate) history_input: Option<u8>,
    pub(crate) history_item: Option<u8>,
    pub(crate) history_match: Option<u8>,
    pub(crate) history_active: Option<u8>,
    pub(crate) history_active_match: Option<u8>,
    pub(crate) error: Option<u8>,
}

pub(crate) struct AppTheme {
    pub(crate) colors: Colors,
}

impl AppTheme {
    pub(crate) fn load(config_path: &Path, config: &Config) -> Result<Self> {
        let colors = match config.theme.as_deref() {
            Some(name) => load_file(config_path, name)?.colors,
            None => Colors::default(),
        };
        Ok(Self { colors })
    }
}

fn load_file(config_path: &Path, name: &str) -> Result<ThemeFile> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("theme must use only letters, numbers, hyphens, and underscores");
    }
    let parent = config_path
        .parent()
        .context("configuration path has no parent directory")?;
    let path = parent.join("themes").join(format!("{name}.toml"));
    let text =
        fs::read_to_string(&path).with_context(|| format!("read theme {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parse theme {}", path.display()))
}

fn write_color(f: &mut dyn fmt::Write, text: &str, color: Option<u8>) -> fmt::Result {
    match color {
        Some(color) => write!(f, "\x1b[38;5;{color}m{text}\x1b[0m"),
        None => write!(f, "{text}"),
    }
}

impl Theme for AppTheme {
    fn format_input_prompt(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        _: Option<&str>,
    ) -> fmt::Result {
        write_color(f, prompt, self.colors.ai_prompt)
    }

    fn format_input_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        selection: &str,
    ) -> fmt::Result {
        write_color(f, prompt, self.colors.ai_prompt)?;
        write_color(f, selection, self.colors.input)
    }

    fn format_select_prompt(&self, f: &mut dyn fmt::Write, prompt: &str) -> fmt::Result {
        write_color(f, prompt, self.colors.menu_prompt)
    }

    fn format_select_prompt_item(
        &self,
        f: &mut dyn fmt::Write,
        text: &str,
        active: bool,
    ) -> fmt::Result {
        write_color(
            f,
            &format!("{} {text}", if active { '›' } else { ' ' }),
            if active {
                self.colors.menu_active
            } else {
                self.colors.menu_item
            },
        )
    }
}

#[cfg(test)]
pub(crate) fn load_for_test(path: &Path, name: &str) -> Result<Colors> {
    Ok(load_file(path, name)?.colors)
}
