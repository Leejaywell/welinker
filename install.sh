#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./install.sh [options]

Build and install the welinker CLI from this source checkout.

Options:
  --prefix PATH      Installation prefix. Defaults to $HOME/.local.
  --bin-dir PATH     Install directory for the welinker binary.
                     Defaults to PREFIX/bin.
  --debug            Build and install the debug binary.
  --no-locked        Do not pass --locked to cargo.
  -h, --help         Show this help.

Environment:
  PREFIX             Default prefix when --prefix is not set.
  CARGO_TARGET_DIR   Cargo target directory.
EOF
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    exit 1
  fi
}

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
prefix="${PREFIX:-$HOME/.local}"
bin_dir=""
profile="release"
locked="--locked"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)
      if [[ $# -lt 2 ]]; then
        echo "error: --prefix requires a path" >&2
        exit 1
      fi
      prefix="$2"
      shift 2
      ;;
    --bin-dir)
      if [[ $# -lt 2 ]]; then
        echo "error: --bin-dir requires a path" >&2
        exit 1
      fi
      bin_dir="$2"
      shift 2
      ;;
    --debug)
      profile="debug"
      shift
      ;;
    --no-locked)
      locked=""
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$bin_dir" ]]; then
  bin_dir="$prefix/bin"
fi

require_cmd cargo
require_cmd npm

cd "$repo_dir"

cargo_args=(build)
if [[ "$profile" == "release" ]]; then
  cargo_args+=(--release)
fi
if [[ -n "$locked" ]]; then
  cargo_args+=("$locked")
fi

echo "Building welinker ($profile)..."
cargo "${cargo_args[@]}"

target_root="${CARGO_TARGET_DIR:-$repo_dir/target}"
if [[ "$target_root" != /* ]]; then
  target_root="$repo_dir/$target_root"
fi

src="$target_root/$profile/welinker"
dest="$bin_dir/welinker"

if [[ ! -x "$src" ]]; then
  echo "error: built binary not found: $src" >&2
  exit 1
fi

install_binary() {
  install -d -m 0755 "$bin_dir"
  install -m 0755 "$src" "$dest"
}

if install_binary 2>/dev/null; then
  :
elif command -v sudo >/dev/null 2>&1; then
  echo "Installing to $bin_dir requires elevated permissions."
  sudo install -d -m 0755 "$bin_dir"
  sudo install -m 0755 "$src" "$dest"
else
  echo "error: cannot write to $bin_dir and sudo is not available" >&2
  exit 1
fi

echo "Installed welinker to $dest"
if ! command -v welinker >/dev/null 2>&1; then
  cat <<EOF

Add this directory to PATH if needed:
  export PATH="$bin_dir:\$PATH"
EOF
fi
