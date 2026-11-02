#!/bin/sh
# The crate stays general: no word of a product built on it.
set -e
cd "$(dirname "$0")/.."
words=$(grep -rnEiw 'loom|bundles?' src README.md Cargo.toml || true)
[ -z "$words" ] || { echo "$words"; exit 1; }
