use std::{
    env,
    ffi::OsStr,
    fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use dialoguer::Select;
use tracing_subscriber::fmt::{fmt as tracing_fmt, time::UtcTime};

mod cli;
mod config;
mod defaults;
mod history_picker;
mod integration;
mod paths;
mod prompt_history;
mod provider_client;
mod selection;
mod session;
mod theme;

use cli::{Cli, Command, ModelCommand};
use config::Config;
use integration::Shell;
use paths::Paths;
#[cfg(test)]
use paths::{CONFIG_FILE, xdg_config_path, xdg_state_directory};
use prompt_history::{read as read_prompt_history, record as record_prompt};
use provider_client::api_key as load_api_key;
#[cfg(test)]
use provider_client::{
    completion_payload, completion_url, error_detail as provider_error_detail, normalize_command,
};
use selection::{
    Selection, load as load_selection, parse as parse_selection, resolve as resolve_selection,
    save as save_selection, with_session_override,
};
use session::{
    CrosstermTerminal, HistoryStyle, RequestHandler, SessionConfig, SessionResult, run_session,
};
use theme::AppTheme;

const CONFIG_EXAMPLE: &str = include_str!("../../config.example.toml");
const DEFAULT_THEME: &str = include_str!("../../themes/default.toml");

pub async fn app_entry() {
    init_logging();
    if let Err(error) = run().await {
        let detail = format!("{error:#}");
        tracing::error!(error = %detail, "shell-ai failed");
        std::process::exit(error_bucket(&detail).exit_code());
    }
}

#[derive(Debug, PartialEq, Eq)]
#[repr(i32)]
enum ErrorBucket {
    ModelUnavailable = 10,
    RateLimited = 11,
    ApiKey = 12,
    Configuration = 13,
    RequestFailed = 14,
}

impl ErrorBucket {
    fn exit_code(self) -> i32 {
        self as i32
    }
}

fn error_bucket(error: &str) -> ErrorBucket {
    let error = error.to_ascii_lowercase();
    if error.contains("api_key")
        || error.contains("api key")
        || error.contains(" 401 ")
        || error.contains("unauthorized")
        || error.contains("forbidden")
    {
        ErrorBucket::ApiKey
    } else if error.contains("429") || error.contains("too many requests") {
        ErrorBucket::RateLimited
    } else if error.contains("not configured")
        || error.contains("unknown provider")
        || error.contains("no provider configured")
        || error.contains("has no models")
        || error.contains("base_url")
        || error.contains("defaults.")
        || error.contains("parse configuration")
    {
        ErrorBucket::Configuration
    } else if error.contains("model ")
        && (error.contains("archived")
            || error.contains("unavailable")
            || error.contains("not found"))
    {
        ErrorBucket::ModelUnavailable
    } else {
        ErrorBucket::RequestFailed
    }
}

fn init_logging() {
    let Ok(paths) = Paths::new() else {
        return;
    };
    let Some(parent) = paths.log.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(log) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log)
    else {
        return;
    };

    let _ = tracing_fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::ERROR)
        .with_target(false)
        .with_timer(UtcTime::rfc_3339())
        .with_writer(Arc::new(log))
        .try_init();
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::new()?;
    match cli.command.unwrap_or(Command::Ask {
        request: Vec::new(),
    }) {
        Command::ConfigPath => println!("{}", paths.config.display()),
        Command::Install => install(&paths)?,
        Command::Doctor => doctor(&paths)?,
        Command::Init { shell } => init(&paths, shell)?,
        Command::Ask { request } => ask(&paths, &request).await?,
        Command::Exec => execute(&paths).await?,
        Command::Model { command } => model(&paths, command.unwrap_or(ModelCommand::Show))?,
        Command::Menu => menu(&paths)?,
        Command::History => history(&paths)?,
    }
    Ok(())
}

fn install(paths: &Paths) -> Result<()> {
    if ensure_config(&paths.config)? {
        println!("created {}", paths.config.display());
    } else {
        println!("configuration already exists at {}", paths.config.display());
    }
    if ensure_default_theme(&paths.config)? {
        println!("created default theme");
    }
    Ok(())
}

fn doctor(paths: &Paths) -> Result<()> {
    let executable = env::current_exe().context("find the running shell-ai binary")?;
    let path_binary = find_in_path(OsStr::new("shell-ai"), env::var_os("PATH").as_deref());
    let binary_on_path = path_binary
        .as_ref()
        .is_some_and(|path| same_file(path, &executable));

    let mut checks = vec![DoctorCheck::ok(
        "binary",
        format!("running {}", executable.display()),
    )];
    checks.push(match path_binary {
        Some(path) if binary_on_path => DoctorCheck::ok("PATH", format!("{}", path.display())),
        Some(path) => DoctorCheck::fail(
            "PATH",
            format!(
                "shell-ai resolves to {}, not {}",
                path.display(),
                executable.display()
            ),
        ),
        None => DoctorCheck::fail("PATH", "shell-ai is not on PATH"),
    });

    match config::load(&paths.config) {
        Err(error) => checks.push(DoctorCheck::fail(
            "config.toml",
            format!("cannot load {}: {error:#}", paths.config.display()),
        )),
        Ok(config) => {
            let detail = if paths.config.exists() {
                format!("{} provider(s) configured", config.providers.len())
            } else {
                format!(
                    "not found at {}; using {} built-in provider(s)",
                    paths.config.display(),
                    config.providers.len()
                )
            };
            checks.push(DoctorCheck::ok("config.toml", detail));
            check_selection(paths, &config, &mut checks);
        }
    }

    for check in &checks {
        println!("{check}");
    }
    if checks.iter().any(|check| !check.ok) {
        bail!("doctor found configuration problems");
    }
    Ok(())
}

fn check_selection(paths: &Paths, config: &Config, checks: &mut Vec<DoctorCheck>) {
    let state = match load_selection(&paths.state) {
        Ok(state) => state,
        Err(error) => {
            checks.push(DoctorCheck::fail("state.toml", error.to_string()));
            return;
        }
    };
    let (provider_name, model, provider) = match resolve_selection(config, &state) {
        Ok(selection) => selection,
        Err(error) => {
            checks.push(DoctorCheck::fail("active model", error.to_string()));
            return;
        }
    };
    checks.push(DoctorCheck::ok(
        "active model",
        format!("{provider_name}/{model}"),
    ));

    match load_api_key(&provider_name, &provider.api_key) {
        Ok(_) => checks.push(DoctorCheck::ok(
            "API key",
            format!("{} returned a key", provider.api_key.description()),
        )),
        Err(error) => checks.push(DoctorCheck::fail("API key", error.to_string())),
    }
}

struct DoctorCheck {
    name: String,
    detail: String,
    ok: bool,
}

impl DoctorCheck {
    fn ok(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            detail: detail.into(),
            ok: true,
        }
    }

    fn fail(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            detail: detail.into(),
            ok: false,
        }
    }
}

impl fmt::Display for DoctorCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.ok { "OK" } else { "FAIL" };
        write!(f, "{status:<4} {:<13} {}", self.name, self.detail)
    }
}

fn find_in_path(name: &OsStr, path: Option<&OsStr>) -> Option<PathBuf> {
    env::split_paths(path?)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file() && is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn same_file(first: &Path, second: &Path) -> bool {
    match (fs::canonicalize(first), fs::canonicalize(second)) {
        (Ok(first), Ok(second)) => first == second,
        _ => false,
    }
}

fn init(paths: &Paths, shell: Shell) -> Result<()> {
    let config = config::load(&paths.config)?;
    print!(
        "{}",
        integration::source(shell, &config.defaults.launch_key)
    );
    Ok(())
}

fn ensure_config(path: &Path) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    let Some(parent) = path.parent() else {
        bail!("configuration path has no parent directory");
    };
    fs::create_dir_all(parent)
        .with_context(|| format!("create configuration directory {}", parent.display()))?;
    fs::write(path, CONFIG_EXAMPLE)
        .with_context(|| format!("write example configuration to {}", path.display()))?;
    Ok(true)
}

fn ensure_default_theme(config_path: &Path) -> Result<bool> {
    let parent = config_path
        .parent()
        .context("configuration path has no parent directory")?;
    let path = parent.join("themes/default.toml");
    if path.exists() {
        return Ok(false);
    }
    fs::create_dir_all(parent.join("themes"))?;
    fs::write(&path, DEFAULT_THEME).with_context(|| format!("write theme {}", path.display()))?;
    Ok(true)
}

async fn execute(paths: &Paths) -> Result<()> {
    let config = config::load(&paths.config)?;
    let mut state = load_selection(&paths.state)?;
    let theme = AppTheme::load(&paths.config, &config)?;
    let mut history = read_prompt_history(&paths.prompt_history)?;
    let history_len = history.len();
    let session_config = SessionConfig {
        prompt: config.defaults.prompt.clone(),
        prompt_color: theme.colors.ai_prompt,
        input_color: theme.colors.input,
        error_color: theme.colors.error,
        history_key: config
            .defaults
            .history_key
            .chars()
            .next()
            .expect("validated history key"),
        menu_key: config
            .defaults
            .menu_key
            .chars()
            .next()
            .expect("validated menu key"),
        history_style: HistoryStyle {
            prompt: theme.colors.history_prompt,
            input: theme.colors.history_input,
            item: theme.colors.history_item,
            matched: theme.colors.history_match,
            active: theme.colors.history_active,
            active_match: theme.colors.history_active_match,
        },
        menu_style: session::MenuStyle {
            prompt: theme.colors.menu_prompt,
            item: theme.colors.menu_item,
            active: theme.colors.menu_active,
        },
    };
    let mut terminal = CrosstermTerminal::new()?;
    let mut handler = ProviderRequestHandler {
        config: &config,
        state_path: &paths.state,
    };

    let result = run_session(
        &session_config,
        &mut state,
        &mut history,
        &mut terminal,
        &mut handler,
    )
    .await;
    terminal.finish()?;
    for prompt in &history[history_len..] {
        record_prompt(&paths.prompt_history, prompt)?;
    }
    match result? {
        SessionResult::Cancelled => Ok(()),
        SessionResult::Command(command) => {
            println!("{command}");
            Ok(())
        }
    }
}

struct ProviderRequestHandler<'a> {
    config: &'a Config,
    state_path: &'a Path,
}

impl RequestHandler<Selection> for ProviderRequestHandler<'_> {
    async fn suggest(&mut self, request: &str, state: &Selection) -> Result<String> {
        request_command_with_state(self.config, state, &[request.to_owned()])
            .await
            .map_err(|error| {
                let detail = format!("{error:#}");
                tracing::error!(error = %detail, "shell-ai request failed");
                anyhow::anyhow!(session_error(&detail))
            })
    }

    fn provider_options(&self) -> Vec<String> {
        self.config.providers.keys().cloned().collect()
    }

    fn model_options(&self, provider: &str) -> Vec<String> {
        self.config
            .providers
            .get(provider)
            .map(|details| details.models.clone())
            .unwrap_or_default()
    }

    fn current_model(&self, state: &Selection) -> Result<String> {
        let (provider, model, _) = resolve_selection(self.config, state)?;
        Ok(format!("{provider}/{model}"))
    }

    fn select_model(&mut self, state: &mut Selection, value: &str) -> Result<()> {
        let (provider, model) = parse_selection(self.config, value)?;
        *state = Selection {
            provider: Some(provider),
            model: Some(model),
        };
        save_selection(self.state_path, state)
    }
}

fn session_error(detail: &str) -> &'static str {
    match error_bucket(detail) {
        ErrorBucket::ModelUnavailable => "model unavailable; select another model",
        ErrorBucket::RateLimited => "provider is busy; try again soon",
        ErrorBucket::ApiKey => "API key problem; check your provider setup",
        ErrorBucket::Configuration => "configuration problem; run shell-ai doctor",
        ErrorBucket::RequestFailed => "request failed; check the log",
    }
}

fn history(paths: &Paths) -> Result<()> {
    let prompts = read_prompt_history(&paths.prompt_history)?;
    if prompts.is_empty() {
        return Ok(());
    }

    let config = config::load(&paths.config)?;
    let theme = AppTheme::load(&paths.config, &config)?;
    if let Some(prompt) = history_picker::pick(
        &prompts,
        [
            theme.colors.history_prompt,
            theme.colors.history_input,
            theme.colors.history_item,
            theme.colors.history_match,
            theme.colors.history_active,
            theme.colors.history_active_match,
        ],
    )? {
        print!("{prompt}");
    }
    Ok(())
}

async fn ask(paths: &Paths, request: &[String]) -> Result<()> {
    let config = config::load(&paths.config)?;
    let stored_state = load_selection(&paths.state)?;
    let session_model = env::var("SHELL_AI_SESSION_MODEL").ok();
    let state = with_session_override(&config, &stored_state, session_model.as_deref())?;
    record_prompt(&paths.prompt_history, &request.join(" "))?;
    ask_with_state(&config, &state, request).await
}

async fn ask_with_state(config: &Config, state: &Selection, request: &[String]) -> Result<()> {
    println!(
        "{}",
        request_command_with_state(config, state, request).await?
    );
    Ok(())
}

async fn request_command_with_state(
    config: &Config,
    state: &Selection,
    request: &[String],
) -> Result<String> {
    let request = if request.is_empty() {
        bail!("provide a request, for example: shell-ai ask 'find large files'");
    } else {
        request.join(" ")
    };
    let (provider_name, model, provider) = resolve_selection(config, state)?;
    let settings = config.request_settings(provider, &model);
    let shell = current_shell()?;
    provider_client::suggest(
        &provider_name,
        provider,
        &model,
        config.defaults.timeout_seconds,
        &settings,
        &shell,
        &request,
    )
    .await
}

fn current_shell() -> Result<String> {
    let shell = env::var("SHELL_AI_SHELL")
        .context("SHELL_AI_SHELL is not set; use a generated shell integration")?;
    shell_from_environment(&shell)
}

fn shell_from_environment(shell: &str) -> Result<String> {
    let shell = shell
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        .collect::<String>();
    if shell.is_empty() {
        bail!("SHELL_AI_SHELL must specify a shell")
    }
    Ok(shell)
}

fn menu(paths: &Paths) -> Result<()> {
    let config = config::load(&paths.config)?;
    let theme = AppTheme::load(&paths.config, &config)?;
    let items = ["model"];
    loop {
        let Some(index) = Select::with_theme(&theme)
            .items(&items)
            .default(0)
            .report(false)
            .interact_opt()?
        else {
            return Ok(());
        };

        match items[index] {
            "model" => {
                if let Some(state) = select_model(paths, &config, &theme)? {
                    println!("{}/{}", state.provider.unwrap(), state.model.unwrap());
                    return Ok(());
                }
            }
            _ => unreachable!("the command menu only contains configured commands"),
        }
    }
}

fn model(paths: &Paths, command: ModelCommand) -> Result<()> {
    let config = config::load(&paths.config)?;
    let theme = AppTheme::load(&paths.config, &config)?;
    match command {
        ModelCommand::Show => {
            let state = load_selection(&paths.state)?;
            let (provider, model, _) = resolve_selection(&config, &state)?;
            println!("{provider}/{model}");
        }
        ModelCommand::List => {
            for (name, provider) in &config.providers {
                for model in &provider.models {
                    println!("{name}/{model}");
                }
            }
        }
        ModelCommand::Use { selection: value } => {
            let (provider, model) = parse_selection(&config, &value)?;
            save_selection(
                &paths.state,
                &Selection {
                    provider: Some(provider.clone()),
                    model: Some(model.clone()),
                },
            )?;
            println!("{provider}/{model}");
        }
        ModelCommand::Select => {
            if let Some(state) = select_model(paths, &config, &theme)? {
                println!("{}/{}", state.provider.unwrap(), state.model.unwrap());
            }
        }
    }
    Ok(())
}

fn select_model(paths: &Paths, config: &Config, theme: &AppTheme) -> Result<Option<Selection>> {
    let items: Vec<String> = config
        .providers
        .iter()
        .flat_map(|(provider, value)| {
            value
                .models
                .iter()
                .map(move |model| format!("{provider}/{model}"))
        })
        .collect();
    if items.is_empty() {
        bail!("no models are configured");
    }
    let current = load_selection(&paths.state)
        .ok()
        .and_then(|state| resolve_selection(config, &state).ok())
        .map(|(provider, model, _)| format!("{provider}/{model}"));
    let default = current
        .and_then(|value| items.iter().position(|item| item == &value))
        .unwrap_or(0);
    let Some(index) = Select::with_theme(theme)
        .with_prompt("model:")
        .items(&items)
        .default(default)
        .report(false)
        .interact_opt()?
    else {
        return Ok(None);
    };
    let (provider, model) = parse_selection(config, &items[index])?;
    let state = Selection {
        provider: Some(provider),
        model: Some(model),
    };
    save_selection(&paths.state, &state)?;
    Ok(Some(state))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use dialoguer::theme::Theme;
    use serde_json::json;

    use super::config::{ApiKeySource, Defaults, Provider};
    use super::*;

    fn test_config() -> Config {
        Config {
            theme: None,
            defaults: Defaults {
                provider: Some("cerebras".to_owned()),
                model: Some("gemma".to_owned()),
                timeout_seconds: 30,
                launch_key: config::default_launch_key(),
                history_key: config::default_history_key(),
                menu_key: config::default_menu_key(),
                prompt: config::default_prompt(),
                max_tokens: 256,
                reasoning_effort: None,
                temperature: 0.1,
            },
            providers: BTreeMap::from([(
                "cerebras".to_owned(),
                Provider {
                    base_url: "https://example.test/v1".to_owned(),
                    api_key: ApiKeySource::Environment("CEREBRAS_API_KEY".to_owned()),
                    models: vec!["gemma".to_owned(), "llama".to_owned()],
                    max_tokens: None,
                    reasoning_effort: None,
                    temperature: None,
                    model_settings: BTreeMap::new(),
                },
            )]),
        }
    }

    #[test]
    fn api_key_accepts_an_environment_variable_or_command() {
        let environment: Provider = toml::from_str(
            r#"base_url = "https://example.test/v1"
api_key = "CEREBRAS_API_KEY"
models = ["gemma"]"#,
        )
        .unwrap();
        assert!(matches!(
            environment.api_key,
            ApiKeySource::Environment(ref name) if name == "CEREBRAS_API_KEY"
        ));

        let command: Provider = toml::from_str(
            r#"base_url = "https://example.test/v1"
api_key = ["fnox", "get", "CEREBRAS_API_KEY"]
models = ["gemma"]"#,
        )
        .unwrap();
        assert!(matches!(
            command.api_key,
            ApiKeySource::Command(ref arguments)
                if arguments == &["fnox", "get", "CEREBRAS_API_KEY"]
        ));
    }

    #[test]
    fn command_api_key_returns_its_standard_output_without_the_trailing_newline() {
        let source = ApiKeySource::Command(vec!["printf".to_owned(), "command-key\\n".to_owned()]);

        assert_eq!(load_api_key("test", &source).unwrap(), "command-key");
    }

    #[test]
    fn selects_the_configured_default() {
        let config = test_config();
        let (provider, model, _) = resolve_selection(&config, &Selection::default()).unwrap();

        assert_eq!(provider, "cerebras");
        assert_eq!(model, "gemma");
    }

    #[test]
    fn session_model_overrides_the_persisted_model() {
        let config = test_config();
        let stored_state = Selection {
            provider: Some("cerebras".to_owned()),
            model: Some("gemma".to_owned()),
        };

        let state = with_session_override(&config, &stored_state, Some("cerebras/llama")).unwrap();
        let (_, model, _) = resolve_selection(&config, &state).unwrap();

        assert_eq!(model, "llama");
    }

    #[test]
    fn rejects_a_state_model_not_offered_by_its_provider() {
        let config = test_config();
        let state = Selection {
            provider: Some("cerebras".to_owned()),
            model: Some("unknown".to_owned()),
        };

        assert!(resolve_selection(&config, &state).is_err());
    }

    #[test]
    fn detects_the_shell_from_the_integration() {
        assert_eq!(shell_from_environment("zsh").unwrap(), "zsh");
    }

    #[test]
    fn rejects_an_empty_shell_from_the_integration() {
        assert!(shell_from_environment("").is_err());
    }

    #[test]
    fn completion_payload_uses_the_selected_model() {
        let config = test_config();
        let state = Selection {
            provider: Some("cerebras".to_owned()),
            model: Some("llama".to_owned()),
        };
        let (_, model, _) = resolve_selection(&config, &state).unwrap();

        let settings = config.request_settings(config.providers.get("cerebras").unwrap(), &model);
        let payload = completion_payload(&model, &settings, "bash", "list files");

        assert_eq!(payload["model"], "llama");
        assert_eq!(payload["max_tokens"], 256);
        assert!(payload.get("max_completion_tokens").is_none());
    }

    #[test]
    fn appends_the_completion_path_to_a_base_url() {
        assert_eq!(
            completion_url("https://example.test/v1").unwrap().as_str(),
            "https://example.test/v1/chat/completions"
        );
    }

    #[test]
    fn input_theme_has_no_spacing_or_completion_prompt() {
        let theme = AppTheme {
            colors: theme::Colors::default(),
        };
        let mut prompt = String::new();
        theme
            .format_input_prompt(&mut prompt, "AI› ", None)
            .unwrap();
        assert_eq!(prompt, "AI› ");

        let mut selection = String::new();
        theme
            .format_input_prompt_selection(&mut selection, "AI", "list files")
            .unwrap();
        assert_eq!(selection, "AIlist files");
    }

    #[test]
    fn command_menu_theme_displays_the_requested_labels() {
        let mut prompt = String::new();
        let theme = AppTheme {
            colors: theme::Colors::default(),
        };
        theme.format_select_prompt(&mut prompt, "menu:").unwrap();

        assert_eq!(prompt, "menu:");

        let mut active_item = String::new();
        theme
            .format_select_prompt_item(&mut active_item, "model", true)
            .unwrap();
        assert_eq!(active_item, "› model");

        let mut model_prompt = String::new();
        theme
            .format_select_prompt(&mut model_prompt, "model:")
            .unwrap();
        assert_eq!(model_prompt, "model:");
    }

    #[test]
    fn groups_request_failures_into_ui_error_buckets() {
        assert_eq!(
            error_bucket("provider returned 404 Not Found: Model zai-glm-4.7 is archived"),
            ErrorBucket::ModelUnavailable
        );
        assert_eq!(
            error_bucket("model zai-glm-4.7 is not configured for cerebras"),
            ErrorBucket::Configuration
        );
        assert_eq!(
            error_bucket("provider returned 429 Too Many Requests"),
            ErrorBucket::RateLimited
        );
        assert_eq!(
            error_bucket("api_key command for provider openai exited with exit status: 1"),
            ErrorBucket::ApiKey
        );
    }

    #[test]
    fn reads_openai_and_top_level_provider_error_messages() {
        assert_eq!(
            provider_error_detail(&json!({"error": {"message": "nested error"}})),
            "nested error"
        );
        assert_eq!(
            provider_error_detail(&json!({"message": "top-level error"})),
            "top-level error"
        );
    }

    #[test]
    fn strips_fences_and_lines() {
        assert_eq!(normalize_command("```sh\nls -la\n```").unwrap(), "ls -la");
    }
    #[test]
    fn rejects_controls() {
        assert!(normalize_command("echo ok\u{1b}").is_err());
    }
    #[test]
    fn joins_multiline_response() {
        assert_eq!(
            normalize_command("git status\n--short").unwrap(),
            "git status --short"
        );
    }

    #[test]
    fn uses_the_xdg_configuration_directory() {
        assert_eq!(
            xdg_config_path(Some(OsStr::new("/tmp/config")), Path::new("/home/tester")),
            PathBuf::from("/tmp/config/shell-ai/config.toml")
        );
        assert_eq!(
            xdg_config_path(None, Path::new("/home/tester")),
            PathBuf::from("/home/tester/.config/shell-ai/config.toml")
        );
    }

    #[test]
    fn uses_the_xdg_state_directory() {
        assert_eq!(
            xdg_state_directory(Some(OsStr::new("/tmp/state")), Path::new("/home/tester")),
            PathBuf::from("/tmp/state/shell-ai")
        );
        assert_eq!(
            xdg_state_directory(None, Path::new("/home/tester")),
            PathBuf::from("/home/tester/.local/state/shell-ai")
        );
    }

    #[test]
    fn install_creates_the_example_config_without_overwriting_an_existing_file() {
        let path = env::temp_dir().join(format!("shell-ai-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        let config = path.join("nested").join(CONFIG_FILE);

        assert!(ensure_config(&config).unwrap());
        assert_eq!(fs::read_to_string(&config).unwrap(), CONFIG_EXAMPLE);
        assert!(ensure_default_theme(&config).unwrap());
        assert_eq!(
            fs::read_to_string(path.join("nested/themes/default.toml")).unwrap(),
            DEFAULT_THEME
        );
        fs::write(&config, "custom configuration").unwrap();
        assert!(!ensure_config(&config).unwrap());
        assert!(!ensure_default_theme(&config).unwrap());
        assert_eq!(fs::read_to_string(&config).unwrap(), "custom configuration");

        fs::remove_dir_all(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn finds_an_executable_binary_on_path() {
        use std::os::unix::fs::PermissionsExt;

        let directory = env::temp_dir().join(format!("shell-ai-path-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let binary = directory.join("shell-ai");
        fs::write(&binary, "").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            find_in_path(OsStr::new("shell-ai"), Some(directory.as_os_str())),
            Some(binary)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn records_nul_delimited_multiline_prompts() {
        let path = env::temp_dir().join(format!("shell-ai-history-test-{}", std::process::id()));
        let _ = fs::remove_file(&path);

        record_prompt(&path, "first line\nsecond line").unwrap();
        record_prompt(&path, "next prompt").unwrap();
        record_prompt(&path, "first line\nsecond line").unwrap();

        assert_eq!(
            fs::read(&path).unwrap(),
            b"first line\nsecond line\0next prompt\0"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn history_picker_uses_a_small_fixed_viewport() {
        assert_eq!(history_picker::height(), "10");
    }

    #[test]
    fn reads_nul_delimited_multiline_prompt_history() {
        let path =
            env::temp_dir().join(format!("shell-ai-history-read-test-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        fs::write(&path, b"first line\nsecond line\0next prompt\0").unwrap();

        assert_eq!(
            read_prompt_history(&path).unwrap(),
            ["first line\nsecond line", "next prompt"]
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn loads_component_colors_from_the_selected_theme_file() {
        let directory = env::temp_dir().join(format!("shell-ai-theme-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("themes")).unwrap();
        fs::write(
            directory.join("themes/night.toml"),
            "[colors]\nai_prompt = 250\ninput = 252\nmenu_active = 110\nhistory_match = 110\nerror = 203\n",
        )
        .unwrap();

        let theme = theme::load_for_test(&directory.join(CONFIG_FILE), "night").unwrap();
        assert_eq!(theme.ai_prompt, Some(250));
        assert_eq!(theme.input, Some(252));
        assert_eq!(theme.menu_active, Some(110));
        assert_eq!(theme.history_match, Some(110));
        assert_eq!(theme.error, Some(203));
        assert!(theme::load_for_test(&directory.join(CONFIG_FILE), "../night").is_err());

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn configuration_validation_rejects_invalid_provider_and_default_forms() {
        let valid = r#"[defaults]
provider = "test"
model = "model"
launch_key = "?"
history_key = "!"
menu_key = "/"

[providers.test]
base_url = "https://example.test/v1"
api_key = "TEST_API_KEY"
models = ["model"]"#;

        for invalid in [
            valid.replace("provider = \"test\"", "provider = \"missing\""),
            valid.replace("model = \"model\"", "model = \"missing\""),
            valid.replace("https://example.test/v1", "not a URL"),
            valid.replace("api_key = \"TEST_API_KEY\"", "api_key = \"\""),
            valid.replace("models = [\"model\"]", "models = []"),
            valid.replace("history_key = \"!\"", "history_key = \"?\""),
            valid.replace("menu_key = \"/\"", "menu_key = \"?\""),
        ] {
            assert!(config::parse(&invalid).is_err(), "{invalid}");
        }

        assert!(config::parse(valid).is_ok());
    }

    #[test]
    fn example_configuration_parses() {
        config::parse(CONFIG_EXAMPLE).unwrap();
    }

    #[test]
    fn defaults_to_question_mark_launch_key() {
        let config: Config = toml::from_str(
            r#"[defaults]
timeout_seconds = 30

[providers.test]
base_url = "https://example.test/v1"
api_key = "TEST_API_KEY"
models = ["test"]"#,
        )
        .unwrap();

        assert_eq!(config.defaults.launch_key, "?");
        assert_eq!(config.defaults.history_key, "!");
        assert_eq!(config.defaults.menu_key, "/");
        assert_eq!(config.defaults.prompt, "AI› ");
    }

    #[test]
    fn launch_key_must_be_one_printable_ascii_character() {
        assert!(config::is_valid_key("?"));
        assert!(!config::is_valid_key(""));
        assert!(!config::is_valid_key("ai"));
        assert!(!config::is_valid_key("é"));
        assert!(!config::is_valid_key("\n"));
    }

    #[test]
    fn launch_history_and_menu_keys_must_be_distinct() {
        let defaults = Defaults {
            launch_key: "?".to_owned(),
            history_key: "!".to_owned(),
            menu_key: "?".to_owned(),
            ..Defaults::default()
        };

        assert!(!defaults.has_valid_keys());
    }
}
