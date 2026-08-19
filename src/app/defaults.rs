//! Built-in configuration used when no configuration file exists.

use std::collections::BTreeMap;

use super::config::{ApiKeySource, Config, Defaults, Provider};

pub(crate) const PROVIDER: &str = "cerebras";
pub(crate) const MODEL: &str = "gemma-4-31b";
pub(crate) const TIMEOUT_SECONDS: u64 = 30;
pub(crate) const LAUNCH_KEY: &str = "?";
pub(crate) const HISTORY_KEY: &str = "!";
pub(crate) const MENU_KEY: &str = "/";
pub(crate) const PROMPT: &str = "AI› ";
pub(crate) const MAX_TOKENS: u64 = 512;
pub(crate) const REASONING_EFFORT: &str = "low";
pub(crate) const TEMPERATURE: f64 = 0.1;

const CEREBRAS_API_KEY: &str = "CEREBRAS_API_KEY";
const CEREBRAS_BASE_URL: &str = "https://api.cerebras.ai/v1";

/// Returns the complete configuration that permits use with only an API-key
/// environment variable. A configuration file can replace these defaults.
pub(crate) fn config() -> Config {
    Config {
        theme: None,
        defaults: Defaults {
            provider: Some(PROVIDER.to_owned()),
            model: Some(MODEL.to_owned()),
            timeout_seconds: TIMEOUT_SECONDS,
            launch_key: LAUNCH_KEY.to_owned(),
            history_key: HISTORY_KEY.to_owned(),
            menu_key: MENU_KEY.to_owned(),
            prompt: PROMPT.to_owned(),
            max_tokens: MAX_TOKENS,
            reasoning_effort: Some(REASONING_EFFORT.to_owned()),
            temperature: TEMPERATURE,
        },
        providers: BTreeMap::from([(
            PROVIDER.to_owned(),
            Provider {
                base_url: CEREBRAS_BASE_URL.to_owned(),
                api_key: ApiKeySource::Environment(CEREBRAS_API_KEY.to_owned()),
                models: vec![MODEL.to_owned(), "gpt-oss-120b".to_owned()],
                max_tokens: None,
                reasoning_effort: None,
                temperature: None,
                model_settings: BTreeMap::new(),
            },
        )]),
    }
}
