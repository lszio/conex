#!/usr/bin/env bash
# Install a repository-local Rust + protoc toolchain under .toolchain/ (gitignored).
# Pinned versions are recorded in tools/codegen.lock.json.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOOLCHAIN="$ROOT/.toolchain"
PROTOC_VERSION="36.1"
mkdir -p "$TOOLCHAIN"

export CARGO_HOME="$TOOLCHAIN/cargo"
export RUSTUP_HOME="$TOOLCHAIN/rustup"

if [ ! -x "$CARGO_HOME/bin/cargo" ]; then
  echo "installing rustup + stable toolchain into $TOOLCHAIN"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/conex-rustup-init.sh
  sh /tmp/conex-rustup-init.sh -y --default-toolchain stable --profile minimal --no-modify-path
fi

if [ ! -x "$TOOLCHAIN/protoc/bin/protoc" ]; then
  echo "installing protoc $PROTOC_VERSION into $TOOLCHAIN/protoc"
  curl -sSL -o /tmp/conex-protoc.zip \
    "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-x86_64.zip"
  rm -rf "$TOOLCHAIN/protoc"
  mkdir -p "$TOOLCHAIN/protoc"
  if command -v unzip >/dev/null 2>&1; then
    unzip -oq /tmp/conex-protoc.zip -d "$TOOLCHAIN/protoc"
  else
    python3 -c "import zipfile; zipfile.ZipFile('/tmp/conex-protoc.zip').extractall('$TOOLCHAIN/protoc')"
  fi
  chmod +x "$TOOLCHAIN/protoc/bin/protoc"
fi

cat > "$TOOLCHAIN/env.sh" <<ENVEOF
export ROOT="$TOOLCHAIN"
export CARGO_HOME="\$ROOT/cargo"
export RUSTUP_HOME="\$ROOT/rustup"
export PATH="\$CARGO_HOME/bin:\$ROOT/protoc/bin:\$PATH"
export BUN_TMPDIR="\$ROOT/bun/tmp"
export BUN_INSTALL="\$ROOT/bun"
export BUN_INSTALL_CACHE_DIR="\$ROOT/bun/cache"
mkdir -p "\$BUN_TMPDIR" "\$BUN_INSTALL_CACHE_DIR"
ENVEOF

echo "toolchain ready. Run: source .toolchain/env.sh"
