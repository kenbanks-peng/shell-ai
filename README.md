# shell-ai

Easily request AI for a terminal command in the syntax of your current shell.

## Quick start

1. Install the latest release binary.

```sh
curl -fsSL https://raw.githubusercontent.com/kenbanks-peng/shell-ai/main/scripts/install.sh | sh
```

2. Get a Cerebras API key.

Note: see [Configuration](#configuration) for use with other providers.

- Go to the [Cerebras Cloud Console](https://cloud.cerebras.ai/) and sign in or create an account.
- Open **API Keys**, select **Create API Key**, and copy the new key.
- Set the key in your shell configuration:

```sh
export CEREBRAS_API_KEY='your-api-key'
```

3. Integrate with your shell

bash (.bashrc): `eval "$(shell-ai init bash)"`
zsh (.zshrc): `eval "$(shell-ai init zsh)"`
fish (config.fish): `shell-ai init fish | source`
nu (config.nu): `$env.config.keybindings ++= (shell-ai init nu | from json)`

## Use

At an empty prompt, press `?`, enter a request, and shell-ai inserts its suggestion at the prompt.

### Example

If `numbers` contains `one`, `two`, and `three`, ask: **“Print the first item of the numbers array.”** shell-ai suggests the matching command for your shell, and each command prints `one`:

| Shell   | Suggestion             |
| ------- | ---------------------- |
| Bash    | `echo "${numbers[0]}"` |
| Zsh     | `echo "${numbers[1]}"` |
| Fish    | `echo $numbers[1]`     |
| Nushell | `echo $numbers.0`      |

### More interesting keys to use

In the request prompt:

- `Up`/`Down` browses shell-ai prompt history.
- `!` searches prompt history.
- `/` opens the menu, including model selection.
- `Escape` closes a menu or returns to the prompt.

## Configuration

For a more advanced configuration, run `shell-ai install` to generate a configuration file and theme.

Use `shell-ai doctor` to check the installation and configuration.

By default, the configuration file will be installed at `$XDG_CONFIG_HOME/shell-ai/config.toml`, or `~/.config/shell-ai/config.toml` when `XDG_CONFIG_HOME` is not set. Set your own config path using `SHELL_AI_CONFIG`. Use `shell-ai config-path` to print the active path.

### Default keys

| Key            | Default value |
| -------------- | ------------- |
| Launch         | `?`           |
| Prompt history | `!`           |
| Menu           | `/`           |

Edit the generated configuration to change the provider, model, or other settings. For example, the included Cerebras provider uses:

```toml
[defaults]
provider = "cerebras"
model = "gemma-4-31b"

[providers.cerebras]
base_url = "https://api.cerebras.ai/v1"
api_key = "CEREBRAS_API_KEY"
models = ["gemma-4-31b", "gpt-oss-120b"]
```

`api_key` can be an environment-variable name or a command array that prints a key, such as `api_key = ["fnox", "get", "OPENAI_API_KEY"]`.

You can set `prompt`, `launch_key`, `history_key`, `menu_key`, `timeout_seconds`, `max_tokens`, `reasoning_effort`, and `temperature` in `[defaults]`. Provider and model settings override these defaults.

Use `shell-ai model list` to show configured models.

## Development

GitHub Actions checks pull requests and pushes to `main`. To publish, update the version in `Cargo.toml`, merge it to `main`, then push the matching tag:

```sh
git tag -a v0.1.0 -m 'Release v0.1.0'
git push origin v0.1.0
```

The release workflow verifies that the tag matches `Cargo.toml`, builds Linux x86_64 and macOS (Intel and Apple Silicon) archives, generates SHA-256 checksums, and creates the GitHub release.
