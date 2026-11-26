#!/bin/sh
# Lorefile installer for macOS and Linux, for people without Node:
#
#   curl -fsSL https://raw.githubusercontent.com/lorefile/lorefile/main/scripts/install.sh | sh
#
# It downloads the latest Lorefile release built for this computer from GitHub
# Releases, checks its SHA-256 checksum, puts the `lorefile` binary in
# ~/.local/bin (or /usr/local/bin when run as root, e.g. `| sudo sh`), and says
# what it did. Optional settings, given before `sh`:
#
#   LOREFILE_VERSION=0.2.0            that version instead of the latest
#   LOREFILE_INSTALL_DIR=/opt/bin     install there (asks sudo if the folder needs it)
#   LOREFILE_SLIM=1                   the slim build: no DuckDB, about 10 MB instead of 70
#   LOREFILE_REPO=someone/lorefile    download from another GitHub repository (a fork)
#   LOREFILE_DOWNLOAD_URL=https://... a folder holding the release archives (a mirror);
#                                     needs LOREFILE_VERSION
#
#   curl -fsSL .../install.sh | LOREFILE_SLIM=1 sh
#
# On Windows use `npx lorefile`, Docker, or the .zip from the releases page.

set -eu

# The GitHub org/repo that publishes the binaries. Change it here, in
# npm/bin/lorefile.js, npm/package.json and .github/workflows/release.yml together.
DEFAULT_REPO="lorefile/lorefile"

say() { printf '%s\n' "$*"; }
die() { printf 'lorefile install: %s\n' "$*" >&2; exit 1; }
has() { command -v "$1" >/dev/null 2>&1; }

# fetch URL FILE: download URL into FILE, failing on HTTP errors.
fetch() {
  if has curl; then
    curl -fsSL --retry 3 -o "$2" "$1"
  else
    wget -q -O "$2" "$1"
  fi
}

# The latest released version, read from where github.com/<repo>/releases/latest
# redirects (.../releases/tag/v1.2.3), so there is no API call and no rate limit.
latest_version() {
  if has curl; then
    url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest") || return 1
  else
    url=$(wget -S -O /dev/null "https://github.com/$repo/releases/latest" 2>&1 | sed -n 's/^ *[Ll]ocation: *//p' | tail -n 1)
  fi
  url=$(printf '%s' "$url" | tr -d '\r')
  case $url in
    */releases/tag/v*) printf '%s\n' "${url##*/releases/tag/v}" ;;
    *) return 1 ;;
  esac
}

# The release target for this computer (keep in step with release.yml's build matrix).
detect_target() {
  os=$(uname -s)
  arch=$(uname -m)
  case $os in
    Darwin)
      # A shell running under Rosetta reports x86_64 on an Apple-silicon Mac;
      # the native arm64 build is the better one there.
      if [ "$arch" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
        arch=arm64
      fi
      case $arch in
        arm64 | aarch64) echo aarch64-apple-darwin ;;
        x86_64) echo x86_64-apple-darwin ;;
        *) die "there is no Lorefile build for macOS on $arch." ;;
      esac
      ;;
    Linux)
      # The Linux builds need glibc 2.35 or newer; musl (Alpine) cannot run them.
      if ldd --version 2>&1 | grep -qi musl; then
        die "this Linux uses musl (Alpine?), which the prebuilt binaries do not support. Use Docker (ghcr.io/lorefile/lorefile) or: cargo install lorefile --locked"
      fi
      case $arch in
        x86_64 | amd64) echo x86_64-unknown-linux-gnu ;;
        aarch64 | arm64) echo aarch64-unknown-linux-gnu ;;
        *) die "there is no Lorefile build for Linux on $arch. Try: cargo install lorefile --locked" ;;
      esac
      ;;
    *)
      die "there is no Lorefile build for $os here. On Windows use npx lorefile, Docker, or the .zip from https://github.com/$repo/releases"
      ;;
  esac
}

sha256_of() {
  if has sha256sum; then
    sha256sum "$1" | cut -d ' ' -f 1
  elif has shasum; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  else
    die "needs sha256sum or shasum to check the download."
  fi
}

# Everything runs from here, so a half-downloaded copy of this script does nothing.
main() {
  repo="${LOREFILE_REPO:-$DEFAULT_REPO}"
  has curl || has wget || die "needs curl or wget."
  has tar || die "needs tar."

  target=$(detect_target)

  version="${LOREFILE_VERSION:-}"
  version="${version#v}"
  if [ -z "$version" ]; then
    if [ -n "${LOREFILE_DOWNLOAD_URL:-}" ]; then
      die "LOREFILE_DOWNLOAD_URL needs LOREFILE_VERSION too."
    fi
    version=$(latest_version) || die "could not find the latest release of $repo. Set LOREFILE_VERSION, or check https://github.com/$repo/releases"
  fi

  suffix=""
  if [ "${LOREFILE_SLIM:-}" = 1 ]; then suffix="-slim"; fi
  name="lorefile-$version-$target$suffix"
  base="${LOREFILE_DOWNLOAD_URL:-https://github.com/$repo/releases/download/v$version}"
  base="${base%/}"

  tmp=$(mktemp -d 2>/dev/null || mktemp -d -t lorefile)
  trap 'rm -rf "$tmp"' EXIT
  trap 'exit 1' HUP INT TERM

  say "Downloading $name.tar.gz"
  say "  from $base"
  fetch "$base/$name.tar.gz" "$tmp/$name.tar.gz" ||
    die "could not download $base/$name.tar.gz. Is $version a release with a build for $target?"
  fetch "$base/$name.tar.gz.sha256" "$tmp/$name.tar.gz.sha256" ||
    die "could not download the checksum $base/$name.tar.gz.sha256."

  read -r expected _ < "$tmp/$name.tar.gz.sha256" || true
  actual=$(sha256_of "$tmp/$name.tar.gz")
  if [ "$expected" != "$actual" ]; then
    die "checksum mismatch for $name.tar.gz (published $expected, downloaded $actual). Nothing was installed."
  fi
  say "Checksum matches: sha256 $actual"

  tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
  binary="$tmp/$name/lorefile"
  [ -f "$binary" ] || die "$name.tar.gz does not contain $name/lorefile."

  if [ -n "${LOREFILE_INSTALL_DIR:-}" ]; then
    dir="$LOREFILE_INSTALL_DIR"
  elif [ "$(id -u)" = 0 ]; then
    dir=/usr/local/bin
  else
    dir="$HOME/.local/bin"
  fi

  # Use sudo only when the folder needs it and we are not root already.
  sudo=""
  if ! { mkdir -p "$dir" 2>/dev/null && [ -w "$dir" ]; }; then
    if [ "$(id -u)" != 0 ] && has sudo; then
      say "$dir needs administrator rights; asking sudo."
      sudo=sudo
      $sudo mkdir -p "$dir"
    else
      die "cannot write to $dir. Set LOREFILE_INSTALL_DIR to a folder you own."
    fi
  fi

  # Copy beside the old binary, then rename over it: a running lorefile keeps
  # working, and nobody ever sees a half-written file.
  $sudo cp "$binary" "$dir/.lorefile.new"
  $sudo chmod 0755 "$dir/.lorefile.new"
  $sudo mv -f "$dir/.lorefile.new" "$dir/lorefile"

  installed=$("$dir/lorefile" --version) || die "copied the binary to $dir/lorefile, but it does not run on this computer."
  say "Installed $installed ($target$suffix) as $dir/lorefile"
  case ":$PATH:" in
    *":$dir:"*) ;;
    *)
      say ""
      say "$dir is not on your PATH yet. Add this line to your shell profile (~/.zshrc, ~/.bashrc or ~/.profile):"
      say "  export PATH=\"$dir:\$PATH\""
      ;;
  esac
  say ""
  say "Next: lorefile serve --dir spaces"
  say "The first run prints a login URL with a token; open it once and the browser remembers you."
}

main "$@"
