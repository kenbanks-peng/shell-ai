//! Platform-specific configuration, state, and log paths.

use std::{
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use directories::BaseDirs;

pub const CONFIG_FILE: &str = "config.toml";
const STATE_FILE: &str = "state.toml";
const LOG_FILE: &str = "shell-ai.log";
const PROMPT_HISTORY_FILE: &str = "prompts.history";

pub struct Paths {
    pub config: PathBuf,
    pub state: PathBuf,
    pub log: PathBuf,
    pub prompt_history: PathBuf,
}
pub fn xdg_config_path(xdg_config_home: Option<&OsStr>, home: &Path) -> PathBuf {
    xdg_config_home
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("shell-ai")
        .join(CONFIG_FILE)
}

pub fn xdg_state_directory(xdg_state_home: Option<&OsStr>, home: &Path) -> PathBuf {
    xdg_state_home
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local").join("state"))
        .join("shell-ai")
}

impl Paths {
    pub fn new() -> Result<Self> {
        let base = BaseDirs::new().context("could not determine home directory")?;
        let config = env::var_os("SHELL_AI_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                xdg_config_path(env::var_os("XDG_CONFIG_HOME").as_deref(), base.home_dir())
            });
        let state_directory =
            xdg_state_directory(env::var_os("XDG_STATE_HOME").as_deref(), base.home_dir());
        let state = state_directory.join(STATE_FILE);
        let log = state_directory.join(LOG_FILE);
        let prompt_history = state_directory.join(PROMPT_HISTORY_FILE);
        Ok(Self {
            config,
            state,
            log,
            prompt_history,
        })
    }
}
