#!/usr/bin/env sh
# One-step remote installer for taglens + its AI skill:
#   curl -fsSL https://raw.githubusercontent.com/yuxuetr/changelog/main/scripts/get.sh | sh
set -eu

REPO="${TAGLENS_REPO:-yuxuetr/changelog}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.cargo/bin}"
mkdir -p "$INSTALL_DIR"

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin) os_part="apple-darwin" ;;
  Linux)  os_part="unknown-linux-gnu" ;;
  *)      os_part="" ;;
esac
case "$arch" in
  x86_64|amd64)  arch_part="x86_64" ;;
  arm64|aarch64) arch_part="aarch64" ;;
  *)             arch_part="" ;;
esac

installed=0
if [ -n "$os_part" ] && [ -n "$arch_part" ]; then
  target="${arch_part}-${os_part}"
  url="https://github.com/${REPO}/releases/latest/download/taglens-${target}.tar.gz"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  if curl -fsSL "$url" -o "$tmp/taglens.tar.gz" 2>/dev/null; then
    tar -xzf "$tmp/taglens.tar.gz" -C "$tmp"
    bin="$(find "$tmp" -name taglens -type f | head -n 1)"
    if [ -n "$bin" ]; then
      cp "$bin" "$INSTALL_DIR/taglens"
      chmod +x "$INSTALL_DIR/taglens"
      installed=1
      echo "Installed prebuilt binary to $INSTALL_DIR/taglens"
    fi
  fi
fi

if [ "$installed" -eq 0 ]; then
  if command -v cargo >/dev/null 2>&1; then
    echo "Prebuilt binary unavailable; building from source via cargo..."
    cargo install --git "https://github.com/${REPO}" --locked --force
    INSTALL_DIR="$(dirname "$(command -v taglens)")"
  else
    echo "error: no prebuilt release found for ${os}/${arch} and cargo is not installed" >&2
    exit 1
  fi
fi

"$INSTALL_DIR/taglens" --version
"$INSTALL_DIR/taglens" skill
