#!/bin/bash
# Install oms on macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/Mr-Sunglasses/oms/main/install.sh | bash
#
# Downloads the latest release (a universal binary for Apple silicon and Intel)
# into ~/.local/bin. Set OMS_INSTALL_DIR to install somewhere else, or
# OMS_VERSION (e.g. v0.1.0) to pick a release.
set -euo pipefail

REPO="Mr-Sunglasses/oms"
DIR="${OMS_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${OMS_VERSION:-latest}"

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || die "oms only runs on macOS."
command -v git >/dev/null || die "oms needs git. Install it with: xcode-select --install"

if [ "$VERSION" = latest ]; then
  url="https://github.com/$REPO/releases/latest/download/oms-macos-universal.tar.gz"
else
  url="https://github.com/$REPO/releases/download/$VERSION/oms-macos-universal.tar.gz"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Every release publishes a SHA-256 of its archive; refuse anything that
# doesn't match it (a truncated download, or a tampered mirror).
verify() {
  curl -fsSL --connect-timeout 15 --max-time 60 --retry 3 "$url.sha256" -o "$tmp/oms.tar.gz.sha256" ||
    die "could not download the checksum $url.sha256"
  expected=$(cut -d ' ' -f 1 < "$tmp/oms.tar.gz.sha256")
  actual=$(shasum -a 256 "$tmp/oms.tar.gz" | cut -d ' ' -f 1)
  [ -n "$expected" ] && [ "$expected" = "$actual" ] ||
    die "checksum mismatch for $url (expected $expected, got $actual); not installing"
}

say "Downloading oms ($VERSION)..."
if curl -fL --progress-bar --connect-timeout 15 --max-time 300 --retry 3 "$url" -o "$tmp/oms.tar.gz"; then
  verify
  tar -xzf "$tmp/oms.tar.gz" -C "$tmp"
elif command -v cargo >/dev/null; then
  say "No prebuilt binary found, building from source with cargo..."
  cargo install --quiet --git "https://github.com/$REPO" --root "$tmp/build"
  mv "$tmp/build/bin/oms" "$tmp/oms"
else
  die "could not download $url"
fi

mkdir -p "$DIR"
install -m 755 "$tmp/oms" "$DIR/oms"
xattr -d com.apple.quarantine "$DIR/oms" 2>/dev/null || true
say "Installed $("$DIR/oms" --version) to $DIR/oms"

case ":$PATH:" in
  *":$DIR:"*) say "Run: oms" ;;
  *)
    shell_rc="$HOME/.zshrc"
    [ "${SHELL##*/}" = bash ] && shell_rc="$HOME/.bash_profile"
    echo
    echo "$DIR is not on your PATH. Add it with:"
    echo
    echo "  echo 'export PATH=\"$DIR:\$PATH\"' >> $shell_rc && source $shell_rc"
    echo
    echo "Then run: oms"
    ;;
esac
