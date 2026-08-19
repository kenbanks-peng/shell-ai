#!/bin/sh
# Download and install the latest shell-ai release binary.
set -eu

repository='kenbanks-peng/shell-ai'
bin_dir="${SHELL_AI_BIN_DIR:-${XDG_BIN_HOME:-$HOME/.local/bin}}"

case "$(uname -s)" in
  Darwin) platform='apple-darwin' ;;
  Linux) platform='unknown-linux-gnu' ;;
  *)
    printf '%s\n' "shell-ai does not support $(uname -s)." >&2
    exit 1
    ;;
esac

case "$(uname -m)" in
  x86_64|amd64) architecture='x86_64' ;;
  arm64|aarch64) architecture='aarch64' ;;
  *)
    printf '%s\n' "shell-ai does not support $(uname -m)." >&2
    exit 1
    ;;
esac

target="${architecture}-${platform}"
archive="shell-ai-${target}.tar.gz"
base_url="https://github.com/${repository}/releases/latest/download"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/shell-ai.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT HUP INT TERM

if ! command -v curl >/dev/null 2>&1; then
  printf '%s\n' 'shell-ai installation requires curl.' >&2
  exit 1
fi

printf 'Downloading shell-ai for %s...\n' "$target"
curl --fail --location --silent --show-error \
  --output "$work_dir/$archive" "$base_url/$archive"
curl --fail --location --silent --show-error \
  --output "$work_dir/$archive.sha256" "$base_url/$archive.sha256"

if command -v shasum >/dev/null 2>&1; then
  (cd "$work_dir" && shasum -a 256 -c "$archive.sha256")
elif command -v sha256sum >/dev/null 2>&1; then
  (cd "$work_dir" && sha256sum -c "$archive.sha256")
else
  printf '%s\n' 'shell-ai installation requires shasum or sha256sum.' >&2
  exit 1
fi

mkdir -p "$bin_dir"
tar -xzf "$work_dir/$archive" -C "$work_dir"
install -m 0755 "$work_dir/shell-ai" "$bin_dir/shell-ai"
printf 'Installed shell-ai to %s/shell-ai\n' "$bin_dir"

case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) printf 'Add %s to your PATH to run shell-ai.\n' "$bin_dir" ;;
esac
