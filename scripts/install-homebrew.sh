#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/install-homebrew.sh [install.sh options]

Install welinker from this source checkout using Homebrew-managed dependencies.
The binary is installed into "$(brew --prefix)/bin" by default.

Any options after this command are forwarded to ./install.sh.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if ! command -v brew >/dev/null 2>&1; then
  echo "error: Homebrew is required: https://brew.sh/" >&2
  exit 1
fi

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

missing=()
for formula in rust node; do
  if ! brew list --versions "$formula" >/dev/null 2>&1; then
    missing+=("$formula")
  fi
done

if [[ ${#missing[@]} -gt 0 ]]; then
  echo "Installing Homebrew dependencies: ${missing[*]}"
  brew install "${missing[@]}"
fi

brew_prefix="$(brew --prefix)"
exec "$repo_dir/install.sh" --prefix "$brew_prefix" "$@"
