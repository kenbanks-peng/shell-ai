//! Fish shell-editor integration script generation.

const TEMPLATE: &str = r#"set -q SHELL_AI_BIN; or set -g SHELL_AI_BIN shell-ai
set _shell_ai_launch_key __SHELL_AI_LAUNCH_KEY__

function _shell_ai_exec
  set -l buffer (commandline)
  if test -n "$buffer"
    commandline -i "$_shell_ai_launch_key"
    return
  end

  set -l command (SHELL_AI_SHELL=fish "$SHELL_AI_BIN" exec </dev/tty)
  set -l exit_code $status
  if test $exit_code -eq 0; and test -n "$command"
    commandline --replace -- "$command"
  end
  commandline -f repaint
end

bind "$_shell_ai_launch_key" _shell_ai_exec
bind -M insert "$_shell_ai_launch_key" _shell_ai_exec
"#;

pub(super) fn source(launch_key: &str) -> String {
    TEMPLATE.replace("__SHELL_AI_LAUNCH_KEY__", &single_quote(launch_key))
}

fn single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_a_quote_for_the_fish_key_sequence() {
        assert!(source("\"").contains("set _shell_ai_launch_key '\"'"));
    }
}
