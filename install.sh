#!/bin/bash
# Install oms on macOS or Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/Mr-Sunglasses/oms/main/install.sh | bash
#
# Downloads the latest release (a universal binary on macOS, an x86_64 or
# aarch64 binary on Linux) into ~/.local/bin. Set OMS_INSTALL_DIR to install
# somewhere else, or OMS_VERSION (e.g. v0.1.0) to pick a release.
set -euo pipefail

REPO="Mr-Sunglasses/oms"
DIR="${OMS_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${OMS_VERSION:-latest}"

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

os=$(uname -s)
arch=$(uname -m)
case "$os" in
  Darwin)
    asset="oms-macos-universal.tar.gz"
    command -v git >/dev/null || die "oms needs git. Install it with: xcode-select --install"
    ;;
  Linux)
    case "$arch" in
      x86_64 | amd64) asset="oms-linux-x86_64.tar.gz" ;;
      aarch64 | arm64) asset="oms-linux-aarch64.tar.gz" ;;
      *) asset="" ;;
    esac
    command -v git >/dev/null || die "oms needs git. Install it with your package manager (e.g. sudo apt install git)"
    ;;
  *) die "oms runs on macOS and Linux, not $os." ;;
esac

if [ "$VERSION" = latest ]; then
  url="https://github.com/$REPO/releases/latest/download/$asset"
else
  url="https://github.com/$REPO/releases/download/$VERSION/$asset"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# macOS ships shasum; most Linux systems have sha256sum instead.
sha256() {
  if command -v shasum >/dev/null; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  else
    sha256sum "$1" | cut -d ' ' -f 1
  fi
}

# Every release publishes a SHA-256 of its archive; refuse anything that
# doesn't match it (a truncated download, or a tampered mirror).
verify() {
  curl -fsSL --connect-timeout 15 --max-time 60 --retry 3 "$url.sha256" -o "$tmp/oms.tar.gz.sha256" ||
    die "could not download the checksum $url.sha256"
  expected=$(cut -d ' ' -f 1 < "$tmp/oms.tar.gz.sha256")
  actual=$(sha256 "$tmp/oms.tar.gz")
  if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
    die "checksum mismatch for $url (expected $expected, got $actual); not installing"
  fi
}

say "Downloading oms ($VERSION)..."
if [ -n "$asset" ] && curl -fL --progress-bar --connect-timeout 15 --max-time 300 --retry 3 "$url" -o "$tmp/oms.tar.gz"; then
  verify
  tar -xzf "$tmp/oms.tar.gz" -C "$tmp"
elif command -v cargo >/dev/null; then
  say "No prebuilt binary found, building from source with cargo..."
  cargo install --quiet --git "https://github.com/$REPO" --root "$tmp/build"
  mv "$tmp/build/bin/oms" "$tmp/oms"
else
  die "could not download oms for $os/$arch; install Rust (https://rustup.rs) and run this again to build it"
fi

mkdir -p "$DIR"
install -m 755 "$tmp/oms" "$DIR/oms"
if [ "$os" = Darwin ]; then
  xattr -d com.apple.quarantine "$DIR/oms" 2>/dev/null || true
fi
say "Installed $("$DIR/oms" --version) to $DIR/oms"

case ":$PATH:" in
  *":$DIR:"*) say "Run: oms" ;;
  *)
    case "${SHELL##*/}" in
      bash)
        shell_rc="$HOME/.bashrc"
        [ "$os" = Darwin ] && shell_rc="$HOME/.bash_profile"
        ;;
      *) shell_rc="$HOME/.zshrc" ;;
    esac
    echo
    echo "$DIR is not on your PATH. Add it with:"
    echo
    echo "  echo 'export PATH=\"$DIR:\$PATH\"' >> $shell_rc && source $shell_rc"
    echo
    echo "Then run: oms"
    ;;
esac
