#!/bin/sh
# Installs odi from a GitHub release: the build for this machine, checked
# against the release's SHA256SUMS.
#
#   curl -fsSL https://github.com/manoj1604/odi/releases/latest/download/install.sh | sh
#
# ODI_VERSION=v0.1.0   a version (default: the latest release)
# ODI_INSTALL=DIR      where to put odi (default: ~/.local/bin, or
#                      /usr/local/bin when run as root)
# ODI_DOWNLOAD=URL     download from here instead (a mirror; with ODI_VERSION)
set -eu

REPO=manoj1604/odi
say() { printf 'odi install: %s\n' "$*" >&2; }
die() { say "$*"; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "needs $1"; }

need uname
need tar
if command -v curl >/dev/null 2>&1; then
  get() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  get() { wget -q "$1" -O "$2"; }
else
  die "needs curl or wget"
fi

case "$(uname -s)" in
  Linux) os=unknown-linux-musl ;;
  Darwin) os=apple-darwin ;;
  *) die "no build for $(uname -s) yet (Linux and macOS): build from source with cargo install --git https://github.com/$REPO odi" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) die "no build for $(uname -m) yet (x86_64 and arm64)" ;;
esac
target=$arch-$os

if [ -n "${ODI_DOWNLOAD:-}" ]; then
  [ -n "${ODI_VERSION:-}" ] || die "ODI_DOWNLOAD needs ODI_VERSION"
  base=$ODI_DOWNLOAD
elif [ -z "${ODI_VERSION:-}" ]; then
  base=https://github.com/$REPO/releases/latest/download
  # (The asset names hold the version: the latest one's, from where the
  # latest release redirects.)
  need curl
  ODI_VERSION=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" | sed 's|.*/tag/||')
  [ -n "$ODI_VERSION" ] || die "couldn't find the latest release"
else
  base=https://github.com/$REPO/releases/download/$ODI_VERSION
fi
name=odi-$ODI_VERSION-$target

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
say "downloading $name"
get "$base/$name.tar.gz" "$tmp/$name.tar.gz" || die "no $name.tar.gz in release $ODI_VERSION"
get "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "no SHA256SUMS in release $ODI_VERSION"

want=$(grep " $name.tar.gz\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
[ -n "$want" ] || die "$name.tar.gz isn't in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
  have=$(sha256sum "$tmp/$name.tar.gz" | cut -d' ' -f1)
else
  have=$(shasum -a 256 "$tmp/$name.tar.gz" | cut -d' ' -f1)
fi
[ "$want" = "$have" ] || die "checksum mismatch for $name.tar.gz: refusing to install"

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
if [ -n "${ODI_INSTALL:-}" ]; then
  dir=$ODI_INSTALL
elif [ "$(id -u)" = 0 ]; then
  dir=/usr/local/bin
else
  dir=$HOME/.local/bin
fi
mkdir -p "$dir"
install -m 0755 "$tmp/$name/odi" "$dir/odi" 2>/dev/null || { cp "$tmp/$name/odi" "$dir/odi" && chmod 0755 "$dir/odi"; }
say "installed $("$dir/odi" version) to $dir/odi"

case ":$PATH:" in
  *":$dir:"*) ;;
  *) say "$dir isn't on your PATH: add  export PATH=\"$dir:\$PATH\"  to your shell's startup file" ;;
esac
if [ "$os" = unknown-linux-musl ] && ! command -v fusermount3 >/dev/null 2>&1; then
  say "to run files from a registry as they're read, install FUSE 3 (Debian/Ubuntu: apt install fuse3)"
fi
