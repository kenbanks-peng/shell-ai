//! Bash Readline integration script generation.

const TEMPLATE: &str = r#": "${SHELL_AI_BIN:=shell-ai}"
_shell_ai_launch_key=__SHELL_AI_LAUNCH_KEY__

_shell_ai_exec() {
  if [[ -n $READLINE_LINE ]]; then
    READLINE_LINE="${READLINE_LINE:0:READLINE_POINT}${_shell_ai_launch_key}${READLINE_LINE:READLINE_POINT}"
    ((READLINE_POINT += ${#_shell_ai_launch_key}))
    return
  fi

  local command exit_code
  command=$(SHELL_AI_SHELL=bash "$SHELL_AI_BIN" exec </dev/tty)
  exit_code=$?
  if (( exit_code == 0 )) && [[ -n $command ]]; then
    READLINE_LINE=$command
    READLINE_POINT=${#READLINE_LINE}
  fi
}

bind -x __SHELL_AI_BINDING__
"#;

pub(super) fn source(launch_key: &str) -> String {
    let binding = format!(
        "\"{}\":_shell_ai_exec",
        launch_key.replace('\\', "\\\\").replace('"', "\\\"")
    );
    TEMPLATE
        .replace("__SHELL_AI_LAUNCH_KEY__", &single_quote(launch_key))
        .replace("__SHELL_AI_BINDING__", &single_quote(&binding))
}

fn single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_a_quote_for_the_readline_key_sequence() {
        assert!(source("\"").contains("bind -x '\"\\\"\":_shell_ai_exec'"));
    }
}
