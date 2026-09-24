#!/usr/bin/env bash
# Run NovaShell windowed with verbose logging (stderr + log file).
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo run -- --windowed --debug "$@"