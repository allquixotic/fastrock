#!/usr/bin/env bash
# Fast compile only. Never opens a GUI on the Mac.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 dev/check-build-policy.py
cargo build --locked --bin fastrock "$@"
