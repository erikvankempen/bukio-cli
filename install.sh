#!/bin/sh
# Install the bukio CLI — a single native binary, no toolchain, no Node.
#
#   curl -fsSL https://raw.githubusercontent.com/erikvankempen/bukio-cli/main/install.sh | sh
#
# Options:
#   --version <tag>   install a specific release (default: the latest)
#   --prefix <dir>    install into <dir> (default: $HOME/.local/bin)
#   --system          install into /usr/local/bin (uses sudo -n when needed)
#   --help
#
# Non-interactive and idempotent on purpose: re-running it upgrades in place.
set -eu

REPO="erikvankempen/bukio-cli"
PREFIX="${HOME:-/root}/.local/bin"
TAG=""
SYSTEM=0

while [ $# -gt 0 ]; do
    case "$1" in
        --version) TAG="${2:-}"; shift 2 ;;
        --prefix)  PREFIX="${2:-}"; shift 2 ;;
        --system)  SYSTEM=1; PREFIX="/usr/local/bin"; shift ;;
        -h|--help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "install.sh: unknown option: $1" >&2; exit 2 ;;
    esac
done

die() { echo "install.sh: $*" >&2; exit 1; }

# ── which artifact: OS, CPU, and the C library ───────────────────────────────
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Linux)
        # glibc and musl are not interchangeable: pick the build that matches the
        # host, so an Alpine container gets the static musl binary.
        if ls /lib/ld-musl-* >/dev/null 2>&1; then libc="musl"; else libc="gnu"; fi
        case "$arch" in
            x86_64|amd64)  triple="x86_64-unknown-linux-${libc}" ;;
            aarch64|arm64) triple="aarch64-unknown-linux-${libc}" ;;
            *) die "unsupported architecture: $arch" ;;
        esac
        ;;
    Darwin)
        case "$arch" in
            arm64)  triple="aarch64-apple-darwin" ;;
            x86_64) triple="x86_64-apple-darwin" ;;
            *) die "unsupported architecture: $arch" ;;
        esac
        ;;
    *) die "unsupported OS: $os (see the README for the supported platforms)" ;;
esac

asset="bukio-${triple}"
if [ -n "$TAG" ]; then
    base="https://github.com/${REPO}/releases/download/${TAG}"
else
    base="https://github.com/${REPO}/releases/latest/download"
fi

# ── fetch ────────────────────────────────────────────────────────────────────
fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        die "neither curl nor wget is available"
    fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "installing bukio (${triple}${TAG:+ ${TAG}})"
fetch "${base}/${asset}" "${tmp}/${asset}" || die "could not download ${base}/${asset} — does that release exist for this platform?"
fetch "${base}/SHA256SUMS" "${tmp}/SHA256SUMS" || die "could not download SHA256SUMS; refusing to install unverified"

# ── verify before anything is written to the install directory ───────────────
expected="$(awk -v a="$asset" '$2 == a {print $1}' "${tmp}/SHA256SUMS")"
[ -n "$expected" ] || die "SHA256SUMS has no entry for ${asset}"

if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${tmp}/${asset}" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "${tmp}/${asset}" | awk '{print $1}')"
elif command -v openssl >/dev/null 2>&1; then
    actual="$(openssl dgst -sha256 "${tmp}/${asset}" | awk '{print $NF}')"
else
    die "no sha256 tool found (need sha256sum, shasum or openssl)"
fi
[ "$actual" = "$expected" ] || die "checksum mismatch for ${asset}: refusing to install"

[ -s "${tmp}/${asset}" ] || die "${asset} is empty"

# ── install ──────────────────────────────────────────────────────────────────
if [ "$SYSTEM" = "1" ] && [ ! -w "$PREFIX" ]; then
    command -v sudo >/dev/null 2>&1 || die "$PREFIX is not writable and sudo is unavailable"
    sudo -n mkdir -p "$PREFIX" 2>/dev/null || die "$PREFIX needs root: re-run with sudo, or drop --system"
    sudo -n install -m 755 "${tmp}/${asset}" "$PREFIX/bukio" || die "could not write to $PREFIX"
else
    mkdir -p "$PREFIX"
    chmod 755 "${tmp}/${asset}"
    mv -f "${tmp}/${asset}" "$PREFIX/bukio"
fi

# macOS quarantines anything downloaded: without this, the first run just says
# the developer cannot be verified.
if [ "$os" = "Darwin" ] && command -v xattr >/dev/null 2>&1; then
    xattr -d com.apple.quarantine "$PREFIX/bukio" 2>/dev/null || true
fi

# ── prove it runs, and say so ────────────────────────────────────────────────
version="$("$PREFIX/bukio" --version 2>/dev/null)" || die "installed, but 'bukio --version' failed to run on this system"
echo "installed bukio ${version} -> ${PREFIX}/bukio"

# Leave a note so `bukio update` knows this is a released binary and not a git
# clone: it then updates itself from the release artifacts instead of asking for
# a toolchain.
cfg="${BUKIO_CONFIG_DIR:-${HOME:-/root}/.bukio}"
mkdir -p "$cfg" 2>/dev/null || true
printf '{\n  "method": "script",\n  "version": "%s",\n  "target": "%s"\n}\n' \
    "$version" "$triple" > "$cfg/install.json" 2>/dev/null || true

case ":${PATH}:" in
    *":${PREFIX}:"*) ;;
    *) echo "add it to your PATH:  export PATH=\"${PREFIX}:\$PATH\"" ;;
esac
