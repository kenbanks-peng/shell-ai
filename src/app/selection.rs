//! Active provider and model selection.

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::config::{Config, Provider};

#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct Selection {
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
}

pub(crate) fn load(path: &Path) -> Result<Selection> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).context("parse model state"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Selection::default()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn save(path: &Path, selection: &Selection) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, toml::to_string(selection)?)?;
    Ok(())
}

pub(crate) fn resolve<'a>(
    config: &'a Config,
    selection: &Selection,
) -> Result<(String, String, &'a Provider)> {
    let provider_name = selection
        .provider
        .as_ref()
        .or(config.defaults.provider.as_ref())
        .or_else(|| config.providers.keys().next())
        .context("no provider configured")?;
    let provider = config
        .providers
        .get(provider_name)
        .with_context(|| format!("unknown provider: {provider_name}"))?;
    let model = selection
        .model
        .as_ref()
        .or(config.defaults.model.as_ref())
        .or_else(|| provider.models.first())
        .context("provider has no models")?;
    if !provider.models.iter().any(|candidate| candidate == model) {
        bail!("model {model} is not configured for provider {provider_name}");
    }
    Ok((provider_name.clone(), model.clone(), provider))
}

pub(crate) fn parse(config: &Config, value: &str) -> Result<(String, String)> {
    let (provider, model) = value
        .split_once('/')
        .context("selection must be PROVIDER/MODEL")?;
    let configured = config
        .providers
        .get(provider)
        .with_context(|| format!("unknown provider: {provider}"))?;
    if !configured.models.iter().any(|candidate| candidate == model) {
        bail!("model {model} is not configured for {provider}");
    }
    Ok((provider.to_owned(), model.to_owned()))
}

pub(crate) fn with_session_override(
    config: &Config,
    stored: &Selection,
    override_value: Option<&str>,
) -> Result<Selection> {
    let Some(value) = override_value.filter(|value| !value.is_empty()) else {
        return Ok(Selection {
            provider: stored.provider.clone(),
            model: stored.model.clone(),
        });
    };
    let (provider, model) = parse(config, value)?;
    Ok(Selection {
        provider: Some(provider),
        model: Some(model),
    })
}
