#!/usr/bin/env bash
# Run NovaShell in a window (developer mode) with default config.
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo run -- --windowed "$@"