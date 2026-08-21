#!/bin/sh
set -eu

output_dir="${1:-public}"

rm -rf "$output_dir"
hugo --environment production --minify --destination "$output_dir"
