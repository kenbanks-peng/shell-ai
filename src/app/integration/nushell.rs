//! Nushell shell-editor integration data generation.

use serde_json::json;

pub(super) fn source(launch_key: &str) -> String {
    let command = format!(
        r#"let buffer = commandline; if not ($buffer | is-empty) {{ commandline edit --insert {launch_key:?} }} else {{ let shell_ai_bin = if "SHELL_AI_BIN" in $env {{ $env.SHELL_AI_BIN }} else {{ "shell-ai" }}; let result = (with-env {{ SHELL_AI_SHELL: "nu" }} {{ do {{ ^$shell_ai_bin exec }} }} | complete); let command = $result.stdout | str trim --right; if $result.exit_code == 0 and not ($command | is-empty) {{ commandline edit --replace $command }} }}"#,
    );
    json!([{
        "name": "shell_ai_exec",
        "modifier": "none",
        "keycode": format!("char_{launch_key}"),
        "mode": ["emacs", "vi_insert"],
        "event": { "send": "executehostcommand", "cmd": command },
    }])
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_a_keybinding_record_that_preserves_quotes() {
        let bindings: serde_json::Value = serde_json::from_str(&source("\"")).unwrap();
        let binding = &bindings[0];

        assert_eq!(binding["keycode"], "char_\"");
        assert_eq!(
            binding["event"]["cmd"].as_str().unwrap(),
            "let buffer = commandline; if not ($buffer | is-empty) { commandline edit --insert \"\\\"\" } else { let shell_ai_bin = if \"SHELL_AI_BIN\" in $env { $env.SHELL_AI_BIN } else { \"shell-ai\" }; let result = (with-env { SHELL_AI_SHELL: \"nu\" } { do { ^$shell_ai_bin exec } } | complete); let command = $result.stdout | str trim --right; if $result.exit_code == 0 and not ($command | is-empty) { commandline edit --replace $command } }"
        );
    }
}
