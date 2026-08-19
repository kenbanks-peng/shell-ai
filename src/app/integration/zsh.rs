//! Zsh shell-editor integration script generation.

const TEMPLATE: &str = r#": "${SHELL_AI_BIN:=shell-ai}"
_shell_ai_launch_key=__SHELL_AI_LAUNCH_KEY__

_shell_ai_exec() {
  if [[ -n $BUFFER && $BUFFER != "$_shell_ai_launch_key" ]]; then
    zle self-insert
    return
  fi

  local command exit_code
  command=$(SHELL_AI_SHELL=zsh "$SHELL_AI_BIN" exec </dev/tty)
  exit_code=$?
  if (( exit_code == 0 )) && [[ -n $command ]]; then
    LBUFFER=$command
  fi
  zle redisplay
}

zle -N _shell_ai_exec
bindkey -M main "$_shell_ai_launch_key" _shell_ai_exec
bindkey -M viins "$_shell_ai_launch_key" _shell_ai_exec
"#;

pub(super) fn source(launch_key: &str) -> String {
    TEMPLATE.replace("__SHELL_AI_LAUNCH_KEY__", &single_quote(launch_key))
}

fn single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
