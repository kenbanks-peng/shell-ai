//! Configuration models and validation.

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    pub(crate) theme: Option<String>,
    #[serde(default)]
    pub(crate) defaults: Defaults,
    pub(crate) providers: BTreeMap<String, Provider>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Defaults {
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    #[serde(default = "default_timeout")]
    pub(crate) timeout_seconds: u64,
    #[serde(default = "default_launch_key")]
    pub(crate) launch_key: String,
    #[serde(default = "default_history_key")]
    pub(crate) history_key: String,
    #[serde(default = "default_menu_key")]
    pub(crate) menu_key: String,
    #[serde(default = "default_prompt")]
    pub(crate) prompt: String,
    #[serde(default = "default_max_tokens")]
    pub(crate) max_tokens: u64,
    pub(crate) reasoning_effort: Option<String>,
    #[serde(default = "default_temperature")]
    pub(crate) temperature: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct RequestSettings {
    pub(crate) max_tokens: Option<u64>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Provider {
    pub(crate) base_url: String,
    pub(crate) api_key: ApiKeySource,
    pub(crate) models: Vec<String>,
    pub(crate) max_tokens: Option<u64>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) temperature: Option<f64>,
    #[serde(flatten)]
    pub(crate) model_settings: BTreeMap<String, RequestSettings>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ApiKeySource {
    Environment(String),
    Command(Vec<String>),
}

impl ApiKeySource {
    pub(crate) fn is_valid(&self) -> bool {
        match self {
            Self::Environment(name) => !name.trim().is_empty(),
            Self::Command(args) => args
                .first()
                .is_some_and(|program| !program.trim().is_empty()),
        }
    }

    pub(crate) fn description(&self) -> String {
        match self {
            Self::Environment(name) => name.clone(),
            Self::Command(args) => args.join(" "),
        }
    }
}

pub(crate) fn load(path: &Path) -> Result<Config> {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let config = super::defaults::config();
            config.validate()?;
            Ok(config)
        }
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

/// Parses and validates configuration for every command entry point.
pub(crate) fn parse(text: &str) -> Result<Config> {
    let config: Config = toml::from_str(text).context("parse configuration")?;
    config.validate()?;
    Ok(config)
}

impl Config {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.providers.is_empty() {
            bail!("configure at least one provider");
        }
        if !self.defaults.has_valid_keys() {
            bail!(
                "defaults.launch_key, defaults.history_key, and defaults.menu_key must be distinct printable ASCII characters"
            );
        }

        for (name, provider) in &self.providers {
            if provider.models.is_empty() {
                bail!("provider {name} has no models");
            }
            super::provider_client::completion_url(&provider.base_url)
                .with_context(|| format!("provider {name} has an invalid base_url"))?;
            if !provider.api_key.is_valid() {
                bail!("provider {name} has an empty api_key");
            }
            validate_request_settings(name, provider)?;
        }
        validate_max_tokens("defaults", self.defaults.max_tokens)?;
        validate_temperature("defaults", self.defaults.temperature)?;
        if let Some(reasoning_effort) = &self.defaults.reasoning_effort {
            validate_reasoning_effort("defaults", reasoning_effort)?;
        }

        let provider_name = self
            .defaults
            .provider
            .as_ref()
            .or_else(|| self.providers.keys().next())
            .expect("validated non-empty providers");
        let provider = self
            .providers
            .get(provider_name)
            .with_context(|| format!("unknown provider: {provider_name}"))?;
        if let Some(model) = &self.defaults.model
            && !provider.models.iter().any(|candidate| candidate == model)
        {
            bail!("model {model} is not configured for provider {provider_name}");
        }
        Ok(())
    }
}

fn default_timeout() -> u64 {
    super::defaults::TIMEOUT_SECONDS
}
fn default_max_tokens() -> u64 {
    super::defaults::MAX_TOKENS
}
fn default_temperature() -> f64 {
    super::defaults::TEMPERATURE
}
pub(crate) fn default_launch_key() -> String {
    super::defaults::LAUNCH_KEY.to_owned()
}
pub(crate) fn default_history_key() -> String {
    super::defaults::HISTORY_KEY.to_owned()
}
pub(crate) fn default_menu_key() -> String {
    super::defaults::MENU_KEY.to_owned()
}
pub(crate) fn default_prompt() -> String {
    super::defaults::PROMPT.to_owned()
}

pub(crate) fn is_valid_key(value: &str) -> bool {
    value.len() == 1 && value.as_bytes()[0].is_ascii_graphic()
}

impl Config {
    pub(crate) fn request_settings(&self, provider: &Provider, model: &str) -> RequestSettings {
        let model_settings = provider.model_settings.get(model);
        RequestSettings {
            max_tokens: model_settings
                .and_then(|settings| settings.max_tokens)
                .or(provider.max_tokens)
                .or(Some(self.defaults.max_tokens)),
            reasoning_effort: model_settings
                .and_then(|settings| settings.reasoning_effort.clone())
                .or_else(|| provider.reasoning_effort.clone())
                .or_else(|| self.defaults.reasoning_effort.clone()),
            temperature: model_settings
                .and_then(|settings| settings.temperature)
                .or(provider.temperature)
                .or(Some(self.defaults.temperature)),
        }
    }
}

fn validate_request_settings(provider_name: &str, provider: &Provider) -> Result<()> {
    if let Some(max_tokens) = provider.max_tokens {
        validate_max_tokens(&format!("provider {provider_name}"), max_tokens)?;
    }
    if let Some(temperature) = provider.temperature {
        validate_temperature(&format!("provider {provider_name}"), temperature)?;
    }
    if let Some(reasoning_effort) = &provider.reasoning_effort {
        validate_reasoning_effort(&format!("provider {provider_name}"), reasoning_effort)?;
    }
    for (model, settings) in &provider.model_settings {
        if !provider.models.iter().any(|configured| configured == model) {
            bail!("provider {provider_name} has settings for an unconfigured model: {model}");
        }
        let location = format!("provider {provider_name}.{model}");
        if let Some(max_tokens) = settings.max_tokens {
            validate_max_tokens(&location, max_tokens)?;
        }
        if let Some(temperature) = settings.temperature {
            validate_temperature(&location, temperature)?;
        }
        if let Some(reasoning_effort) = &settings.reasoning_effort {
            validate_reasoning_effort(&location, reasoning_effort)?;
        }
    }
    Ok(())
}

fn validate_max_tokens(location: &str, max_tokens: u64) -> Result<()> {
    if max_tokens == 0 {
        bail!("{location}.max_tokens must be greater than zero");
    }
    Ok(())
}

fn validate_temperature(location: &str, temperature: f64) -> Result<()> {
    if !temperature.is_finite() {
        bail!("{location}.temperature must be finite");
    }
    Ok(())
}

fn validate_reasoning_effort(location: &str, reasoning_effort: &str) -> Result<()> {
    if reasoning_effort.trim().is_empty() {
        bail!("{location}.reasoning_effort must not be empty");
    }
    Ok(())
}

impl Defaults {
    pub(crate) fn has_valid_keys(&self) -> bool {
        is_valid_key(&self.launch_key)
            && is_valid_key(&self.history_key)
            && is_valid_key(&self.menu_key)
            && self.launch_key != self.history_key
            && self.launch_key != self.menu_key
            && self.history_key != self.menu_key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_built_in_defaults_when_configuration_file_is_absent() {
        let directory = std::env::temp_dir().join(format!(
            "shell-ai-config-defaults-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let config = load(&directory.join("config.toml")).unwrap();

        assert_eq!(config.defaults.provider.as_deref(), Some("cerebras"));
        assert_eq!(config.defaults.model.as_deref(), Some("gemma-4-31b"));
        assert_eq!(config.defaults.max_tokens, 512);
        assert!(config.providers.contains_key("cerebras"));
    }

    #[test]
    fn request_settings_layer_from_defaults_to_provider_to_model() {
        let config = parse(
            r#"
[defaults]
max_tokens = 1024
reasoning_effort = "low"
temperature = 0.1

[providers.cerebras]
base_url = "https://example.test/v1"
api_key = "CEREBRAS_API_KEY"
models = ["gemma", "gpt-oss"]
max_tokens = 512
temperature = 0.2

[providers.cerebras.gpt-oss]
max_tokens = 2048
reasoning_effort = "medium"
"#,
        )
        .unwrap();
        let provider = config.providers.get("cerebras").unwrap();

        let gemma = config.request_settings(provider, "gemma");
        assert_eq!(gemma.max_tokens, Some(512));
        assert_eq!(gemma.reasoning_effort.as_deref(), Some("low"));
        assert_eq!(gemma.temperature, Some(0.2));

        let gpt_oss = config.request_settings(provider, "gpt-oss");
        assert_eq!(gpt_oss.max_tokens, Some(2048));
        assert_eq!(gpt_oss.reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(gpt_oss.temperature, Some(0.2));
    }

    #[test]
    fn request_settings_keep_existing_payload_defaults_when_not_configured() {
        let config = parse(
            r#"
[defaults]

[providers.test]
base_url = "https://example.test/v1"
api_key = "TEST_API_KEY"
models = ["model"]
"#,
        )
        .unwrap();
        let provider = config.providers.get("test").unwrap();
        let settings = config.request_settings(provider, "model");

        assert_eq!(settings.max_tokens, Some(512));
        assert_eq!(settings.temperature, Some(0.1));
        assert_eq!(settings.reasoning_effort, None);
    }
}
