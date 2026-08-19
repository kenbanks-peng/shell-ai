//! OpenAI-compatible provider requests.

use std::{env, process::Command, time::Duration};

use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::json;
use url::Url;

use super::config::{ApiKeySource, Provider, RequestSettings};

const SYSTEM_PROMPT: &str = "Return exactly one shell command that fulfils the user request. Return only the command, with no Markdown, code fences, explanation, or line breaks. The command will be inserted into an interactive shell for review; never state that it was executed.";

/// Requests one command from a configured provider.
pub(crate) async fn suggest(
    provider_name: &str,
    provider: &Provider,
    model: &str,
    timeout_seconds: u64,
    settings: &RequestSettings,
    shell: &str,
    request: &str,
) -> Result<String> {
    let key = api_key(provider_name, &provider.api_key)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()?;
    let response = client
        .post(completion_url(&provider.base_url)?)
        .bearer_auth(key)
        .json(&completion_payload(model, settings, shell, request))
        .send()
        .await
        .context("could not contact inference provider")?;
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .context("provider returned invalid JSON")?;
    if !status.is_success() {
        bail!("provider returned {status}: {}", error_detail(&body));
    }
    let content = body
        .pointer("/choices/0/message/content")
        .and_then(|value| value.as_str())
        .with_context(|| {
            format!(
                "provider response contained no message content ({})",
                response_diagnostics(&body)
            )
        })?;
    normalize_command(content)
}

pub(crate) fn api_key(provider_name: &str, source: &ApiKeySource) -> Result<String> {
    match source {
        ApiKeySource::Environment(key_name) => {
            let key = env::var(key_name).with_context(|| {
                format!("read environment variable {key_name} for provider {provider_name}")
            })?;
            if key.is_empty() {
                bail!("environment variable {key_name} is empty for provider {provider_name}");
            }
            Ok(key)
        }
        ApiKeySource::Command(arguments) => {
            let (program, arguments) = arguments.split_first().with_context(|| {
                format!("api_key command is empty for provider {provider_name}")
            })?;
            if program.trim().is_empty() {
                bail!("api_key command is empty for provider {provider_name}");
            }
            let output = Command::new(program)
                .args(arguments)
                .output()
                .with_context(|| format!("run api_key command for provider {provider_name}"))?;
            if !output.status.success() {
                bail!(
                    "api_key command for provider {provider_name} exited with {}",
                    output.status
                );
            }
            let key = String::from_utf8(output.stdout).with_context(|| {
                format!("read api_key command output for provider {provider_name}")
            })?;
            let key = key.trim_end_matches(['\r', '\n']);
            if key.is_empty() {
                bail!("api_key command returned an empty key for provider {provider_name}");
            }
            Ok(key.to_owned())
        }
    }
}

pub(crate) fn completion_payload(
    model: &str,
    settings: &RequestSettings,
    shell: &str,
    request: &str,
) -> serde_json::Value {
    let mut payload = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt(shell)},
            {"role": "user", "content": request}
        ]
    });
    let object = payload
        .as_object_mut()
        .expect("completion payload is an object");
    if let Some(max_tokens) = settings.max_tokens {
        object.insert("max_tokens".to_owned(), json!(max_tokens));
    }
    if let Some(temperature) = settings.temperature {
        object.insert("temperature".to_owned(), json!(temperature));
    }
    if let Some(reasoning_effort) = &settings.reasoning_effort {
        object.insert("reasoning_effort".to_owned(), json!(reasoning_effort));
    }
    payload
}

fn system_prompt(shell: &str) -> String {
    format!("{SYSTEM_PROMPT} Generate syntax that is compatible with {shell}.")
}

pub(crate) fn completion_url(base_url: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).context("invalid provider base_url")?;
    if !url.path().ends_with('/') {
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("base_url cannot be a base"))?
            .push("");
    }
    url.join("chat/completions")
        .context("construct chat completions URL")
}

pub(crate) fn error_detail(body: &serde_json::Value) -> &str {
    body.pointer("/error/message")
        .or_else(|| body.get("message"))
        .and_then(|value| value.as_str())
        .unwrap_or("unknown provider error")
}

/// Returns response-shape metadata without including provider-supplied text.
fn response_diagnostics(body: &serde_json::Value) -> String {
    let choice = body.pointer("/choices/0");
    let message = choice.and_then(|value| value.get("message"));
    let content = message.and_then(|value| value.get("content"));
    let choice_count = body
        .get("choices")
        .and_then(|value| value.as_array())
        .map_or(0, Vec::len);
    let message_kind = match message {
        None => "missing",
        Some(value) if value.is_object() => "object",
        Some(value) if value.is_null() => "null",
        Some(_) => "other",
    };
    let content_kind = match content {
        None => "missing",
        Some(value) if value.is_string() => "string",
        Some(value) if value.is_null() => "null",
        Some(value) if value.is_array() => "array",
        Some(value) if value.is_object() => "object",
        Some(_) => "other",
    };
    let finish_reason = match choice
        .and_then(|value| value.get("finish_reason"))
        .and_then(|value| value.as_str())
    {
        Some("length") => "length",
        Some("stop") => "stop",
        Some("tool_calls") => "tool_calls",
        Some("content_filter") => "content_filter",
        Some(_) => "other",
        None => "missing",
    };
    let has_field = |name| message.and_then(|value| value.get(name)).is_some();

    format!(
        "choice_count={choice_count} message={message_kind} content={content_kind} finish_reason={finish_reason} refusal={} tool_calls={} reasoning={}",
        has_field("refusal"),
        has_field("tool_calls"),
        has_field("reasoning")
    )
}

pub(crate) fn normalize_command(content: &str) -> Result<String> {
    let mut command = content.trim().trim_matches('`').trim().to_owned();
    if let Some((language, body)) = command.split_once('\n')
        && language
            .bytes()
            .all(|character| character.is_ascii_alphanumeric() || character == b'-')
    {
        command = body.trim().to_owned();
    }
    command = command
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if command.is_empty() {
        bail!("provider returned no command");
    }
    if command.chars().any(|character| character.is_control()) {
        bail!("provider returned unsafe control characters");
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    use super::*;

    struct CapturedRequest {
        path: String,
        authorization: Option<String>,
        payload: serde_json::Value,
    }

    struct MockProvider {
        provider: Provider,
        server: thread::JoinHandle<CapturedRequest>,
    }

    impl MockProvider {
        fn join(self) -> CapturedRequest {
            self.server.join().expect("mock provider thread panicked")
        }
    }

    fn mock_provider(response: &str) -> MockProvider {
        mock_provider_with_delay(response, Duration::ZERO)
    }

    fn mock_slow_provider() -> MockProvider {
        mock_provider_with_delay(
            &http_response("200 OK", r#"{"choices":[{"message":{"content":"ls"}}]}"#),
            Duration::from_secs(2),
        )
    }

    fn mock_provider_with_delay(response: &str, delay: Duration) -> MockProvider {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = response.to_owned();
        let server = thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let request = read_request(&mut connection);
            thread::sleep(delay);
            connection.write_all(response.as_bytes()).unwrap();
            request
        });
        MockProvider {
            provider: Provider {
                base_url: format!("http://{address}/v1"),
                api_key: ApiKeySource::Command(vec!["printf".to_owned(), "test-key".to_owned()]),
                models: vec!["test-model".to_owned()],
                max_tokens: None,
                reasoning_effort: None,
                temperature: None,
                model_settings: Default::default(),
            },
            server,
        }
    }

    fn http_response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    fn read_request(connection: &mut impl Read) -> CapturedRequest {
        let mut bytes = Vec::new();
        let mut buffer = [0; 1024];
        let header_end = loop {
            let count = connection.read(&mut buffer).unwrap();
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break end + 4;
            }
        };
        let (content_length, path, authorization) = {
            let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let path = headers.split_whitespace().nth(1).unwrap().to_owned();
            let authorization = headers
                .lines()
                .find_map(|line| line.strip_prefix("authorization: ").map(str::to_owned));
            (content_length, path, authorization)
        };
        while bytes.len() < header_end + content_length {
            let count = connection.read(&mut buffer).unwrap();
            bytes.extend_from_slice(&buffer[..count]);
        }

        CapturedRequest {
            path,
            authorization,
            payload: serde_json::from_slice(&bytes[header_end..]).unwrap(),
        }
    }

    #[tokio::test]
    async fn suggest_sends_openai_requests_and_reports_provider_failures() {
        let success = mock_provider(&http_response(
            "200 OK",
            r#"{"choices":[{"message":{"content":"ls -la"}}]}"#,
        ));
        let command = suggest(
            "test",
            &success.provider,
            "test-model",
            1,
            &RequestSettings::default(),
            "bash",
            "list files",
        )
        .await
        .unwrap();
        let request = success.join();

        assert_eq!(command, "ls -la");
        assert_eq!(request.path, "/v1/chat/completions");
        assert_eq!(request.authorization.as_deref(), Some("Bearer test-key"));
        assert_eq!(request.payload["model"], "test-model");
        assert_eq!(
            request.payload["messages"][0]["content"],
            format!("{SYSTEM_PROMPT} Generate syntax that is compatible with bash.")
        );
        assert_eq!(request.payload["messages"][1]["content"], "list files");

        for (response, expected_error) in [
            (
                http_response(
                    "500 Internal Server Error",
                    r#"{"error":{"message":"provider failed"}}"#,
                ),
                "provider returned 500 Internal Server Error: provider failed",
            ),
            (
                http_response("200 OK", "not-json"),
                "provider returned invalid JSON",
            ),
            (
                http_response("200 OK", r#"{"choices":[{}]}"#),
                "provider response contained no message content",
            ),
        ] {
            let mock = mock_provider(&response);
            let error = suggest(
                "test",
                &mock.provider,
                "test-model",
                1,
                &RequestSettings::default(),
                "bash",
                "list files",
            )
            .await
            .unwrap_err();
            mock.join();
            assert!(format!("{error:#}").contains(expected_error));
        }

        let timeout = mock_slow_provider();
        let error = suggest(
            "test",
            &timeout.provider,
            "test-model",
            1,
            &RequestSettings::default(),
            "bash",
            "list files",
        )
        .await
        .unwrap_err();
        timeout.join();
        assert!(format!("{error:#}").contains("could not contact inference provider"));
    }

    #[test]
    fn completion_payload_includes_configured_request_settings() {
        let payload = completion_payload(
            "test-model",
            &RequestSettings {
                max_tokens: Some(1024),
                reasoning_effort: Some("medium".to_owned()),
                temperature: Some(0.2),
            },
            "fish",
            "list files",
        );

        assert_eq!(payload["max_tokens"], 1024);
        assert_eq!(payload["reasoning_effort"], "medium");
        assert_eq!(payload["temperature"], 0.2);
        assert_eq!(
            payload["messages"][0]["content"],
            format!("{SYSTEM_PROMPT} Generate syntax that is compatible with fish.")
        );
    }

    #[test]
    fn response_diagnostics_excludes_provider_text() {
        let body = json!({
            "choices": [{
                "finish_reason": "length",
                "message": {
                    "content": null,
                    "reasoning": "private reasoning",
                    "refusal": "private refusal"
                }
            }]
        });

        assert_eq!(
            response_diagnostics(&body),
            "choice_count=1 message=object content=null finish_reason=length refusal=true tool_calls=false reasoning=true"
        );
    }
}
