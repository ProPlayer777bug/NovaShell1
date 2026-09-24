#!/usr/bin/env bash
# Run all non-GUI tests. Works on the dev machine too (logic only).
set -euo pipefail
cd "$(dirname "$0")/.."

if command -v node >/dev/null 2>&1; then
  echo "==> UI script syntax check"
  if [ -f ui/index.html ]; then
    node -e "const fs=require('fs');const h=fs.readFileSync('ui/index.html','utf8');const m=h.match(/<script>([\s\S]*?)<\/script>/);if(!m){console.error('no inline script found');process.exit(1);}fs.writeFileSync(require('os').tmpdir()+'/nova_ui_check.js',m[1]);" 
    node --check "$(node -e "process.stdout.write(require('os').tmpdir()+'/nova_ui_check.js')")"
    echo "    OK"
  fi
fi

if command -v cargo >/dev/null 2>&1; then
  echo "==> cargo test"
  cargo test --release
else
  echo "==> cargo not available here (this machine built only the JS side)."
  echo "    Run on Ubuntu:  cargo test"
fi