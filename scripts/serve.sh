#!/usr/bin/env bash
# Площадка и прокси: http://localhost:8000/site/playground.html
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f site/playground.html ] || bash scripts/build-site.sh
cargo run -q --release -p vladx-cli -- serve --port "${PORT:-8000}"
