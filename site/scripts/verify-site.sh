#!/bin/sh
set -eu

output_dir="${1:-public}"

rm -rf "$output_dir"
mkdir -p "$output_dir"
printf '%s\n' 'This file must be removed by a clean Hugo build.' > "$output_dir/stale.html"

sh scripts/build-site.sh "$output_dir"

test ! -e "$output_dir/stale.html"
test -f "$output_dir/index.html"
grep -Fq 'shell-ai — Ask your shell' "$output_dir/index.html"
test -f "$output_dir/docs/index.html"
grep -Fq 'Documentation' "$output_dir/docs/index.html"
test -f "$output_dir/integrations/index.html"
grep -Fq 'href=/shell-ai/integrations/' "$output_dir/index.html"
for shell in bash zsh fish nushell; do
  test -f "$output_dir/integrations/$shell/index.html"
  grep -Fq "href=/shell-ai/integrations/$shell/" "$output_dir/integrations/index.html"
done
grep -Fq 'eval &#34;$(shell-ai init bash)&#34;' "$output_dir/integrations/bash/index.html"
grep -Fq 'shell-ai init fish | source' "$output_dir/integrations/fish/index.html"
grep -Fq '$env.config.keybindings &#43;&#43;= (shell-ai init nu | from json)' "$output_dir/integrations/nushell/index.html"

npx --no-install stylelint assets/css/styles.css
