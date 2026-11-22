#!/bin/sh
# Installs or updates Kirie for the current user, without sudo:
#
#   curl -fsSL https://samuelba.dev/kirie/install.sh | sh
#
# Running it again updates to the latest release. Options (after `sh -s --` when piped):
#   --gnome-extension  also install the GNOME Shell extension when not on GNOME right now
#   --force            reinstall even when this version is already installed
#   --uninstall        remove Kirie; the configuration in ~/.config/kirie stays
# Environment: KIRIE_VERSION pins a release (e.g. 0.2.0). KIRIE_GNOME_EXTENSION=1, KIRIE_FORCE=1
# and KIRIE_UNINSTALL=1 do the same as the options. KIRIE_SHA256 is the tarball's checksum,
# which Kirie's updater passes after checking the release's signature.
#
# When minisign or rsign is installed, the download is checked against the release's signed
# SHA256SUMS. Without either, it is checked against the unsigned SHA256SUMS-<arch>.
#
# Files: ~/.local/bin/kirie, the desktop entry, icons and metainfo under ~/.local/share, and the
# GNOME Shell extension under ~/.local/share/gnome-shell/extensions.
set -eu

repo="https://github.com/samuelba/kirie"
app_id="dev.samuelba.Kirie"
uuid="kirie@samuelba.dev"
# The release key's public half, the same as packaging/minisign.pub.
public_key=""
bin_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}"
binary="$bin_dir/kirie"
marker="$data_dir/kirie/install-method"
extension_dir="$data_dir/gnome-shell/extensions/$uuid"
icon_files="scalable/apps/$app_id.svg symbolic/apps/$app_id-symbolic.svg"

gnome_extension="${KIRIE_GNOME_EXTENSION:-0}"
force="${KIRIE_FORCE:-0}"
uninstall="${KIRIE_UNINSTALL:-0}"
for arg in "$@"; do
    case "$arg" in
        --gnome-extension) gnome_extension=1 ;;
        --force) force=1 ;;
        --uninstall) uninstall=1 ;;
        -h | --help)
            echo "Usage: install.sh [--gnome-extension] [--force] [--uninstall]"
            echo "Set KIRIE_VERSION to install a specific release."
            exit 0
            ;;
        *)
            echo "kirie install: unknown option $arg" >&2
            exit 2
            ;;
    esac
done

say() { printf '%s\n' "$*"; }
die() {
    printf 'kirie install: %s\n' "$*" >&2
    exit 1
}
has() { command -v "$1" >/dev/null 2>&1; }

fetch() { # URL FILE
    if has curl; then
        curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"
    elif has wget; then
        wget -q --https-only -O "$2" "$1"
    else
        die "needs curl or wget"
    fi
}

latest_version() {
    if has curl; then
        url="$(curl -fsSLI --proto '=https' -o /dev/null -w '%{url_effective}' "$repo/releases/latest")"
    elif has wget; then
        url="$(wget -q --https-only -S --spider --max-redirect=0 "$repo/releases/latest" 2>&1 |
            sed -n 's/^ *[Ll]ocation: *//p' | tr -d '\r' | tail -n 1)" || true
    else
        die "needs curl or wget"
    fi
    version="${url##*/tag/v}"
    case "$version" in
        "" | */*) die "cannot find the latest release at $repo/releases/latest" ;;
    esac
    printf '%s\n' "$version"
}

installed_version() {
    if [ -f "$marker" ]; then
        sed -n 's/^version=//p' "$marker"
    elif [ -x "$binary" ]; then
        "$binary" --version 2>/dev/null | awk '{ print $2 }'
    fi
}

sha256() {
    if has sha256sum; then
        sha256sum "$1" | awk '{ print $1 }'
    elif has shasum; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        die "needs sha256sum or shasum to check the download"
    fi
}

on_gnome() {
    case ":${XDG_CURRENT_DESKTOP:-}:" in
        *:GNOME:* | *:GNOME-Classic:*) return 0 ;;
    esac
    return 1
}

# Checks SHA256SUMS against SHA256SUMS.minisig: signed by the release key, for this version.
verify_signature() { # DIR
    if has minisign; then
        minisign -V -q -P "$public_key" -m "$1/SHA256SUMS" -x "$1/SHA256SUMS.minisig" >/dev/null
    else
        rsign verify -q -P "$public_key" -x "$1/SHA256SUMS.minisig" "$1/SHA256SUMS" >/dev/null
    fi || die "the checksums of Kirie $version are not signed by the release key. Nothing was installed."
    comment="$(sed -n 's/^trusted comment: //p' "$1/SHA256SUMS.minisig")"
    [ "$comment" = "kirie $version" ] ||
        die "the checksums are signed for \"$comment\", not \"kirie $version\". Nothing was installed."
}

# The expected checksum of the tarball, from the signed SHA256SUMS when it can be checked.
expected_sha256() { # DIR BUNDLE
    if [ -n "${KIRIE_SHA256:-}" ]; then
        printf '%s\n' "$KIRIE_SHA256"
        return 0
    fi
    sums="SHA256SUMS-$arch"
    if [ -n "$public_key" ] && { has minisign || has rsign; }; then
        fetch "$repo/releases/download/v$version/SHA256SUMS" "$1/SHA256SUMS" ||
            die "cannot download the checksums. Nothing was installed."
        fetch "$repo/releases/download/v$version/SHA256SUMS.minisig" "$1/SHA256SUMS.minisig" ||
            die "cannot download the signature. Nothing was installed."
        verify_signature "$1"
        sums="SHA256SUMS"
        say "Signature checked." >&2
    else
        fetch "$repo/releases/download/v$version/$sums" "$1/$sums" ||
            die "cannot download the checksums. Nothing was installed."
    fi
    awk -v name="$2.tar.gz" '$2 == name || $2 == "*" name { print $1 }' "$1/$sums"
}

# The Exec key of a desktop entry. Paths with characters that need escaping are refused.
desktop_exec() {
    case "$1" in
        *[\"\`\$\\]*) die "cannot write a desktop entry for $1 (it contains \", \`, \$ or \\)" ;;
        *[!A-Za-z0-9/._+-]*) printf '"%s"' "$1" ;;
        *) printf '%s' "$1" ;;
    esac
}

# Other installers may have left an icon cache here, which hides icons it does not list.
refresh_icon_cache() {
    icon_dir="$data_dir/icons/hicolor"
    [ -f "$icon_dir/icon-theme.cache" ] || return 0
    for tool in gtk-update-icon-cache gtk4-update-icon-cache; do
        if has "$tool"; then
            "$tool" -q -f -t "$icon_dir" 2>/dev/null && return 0
        fi
    done
    rm -f -- "$icon_dir/icon-theme.cache"
}

do_uninstall() {
    [ -e "$binary" ] || [ -e "$marker" ] || die "Kirie is not installed in $bin_dir"
    if [ -d "$extension_dir" ]; then
        if has gnome-extensions; then
            gnome-extensions disable "$uuid" 2>/dev/null || true
        fi
        rm -rf -- "$extension_dir"
    fi
    rm -f -- "$binary" "$marker" \
        "$data_dir/applications/$app_id.desktop" \
        "$data_dir/metainfo/$app_id.metainfo.xml"
    for icon in $icon_files; do
        rm -f -- "$data_dir/icons/hicolor/$icon"
    done
    refresh_icon_cache
    rmdir -- "$data_dir/kirie" 2>/dev/null || true
    say "Removed Kirie. Your configuration in $config_dir/kirie stays."
}

[ "$(uname -s)" = Linux ] || die "Kirie runs on Linux only"

if [ "$uninstall" = 1 ]; then
    do_uninstall
    exit 0
fi

if [ -e /usr/bin/kirie ]; then
    die "Kirie is installed as a system package (/usr/bin/kirie). Update it with your package manager, or remove the package before using this script."
fi

case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) die "there is no Kirie build for $(uname -m), only for x86_64 and aarch64" ;;
esac

if [ -n "${KIRIE_VERSION:-}" ]; then
    version="${KIRIE_VERSION#v}"
else
    version="$(latest_version)"
fi
previous="$(installed_version || true)"

# A GNOME extension installed earlier is kept up to date, even when updating from another desktop.
want_extension=0
if [ "$gnome_extension" = 1 ] || on_gnome || [ -d "$extension_dir" ]; then
    want_extension=1
fi

if [ "$previous" = "$version" ] && [ "$force" != 1 ]; then
    if [ "$gnome_extension" = 1 ] && [ ! -d "$extension_dir" ]; then
        "$binary" gnome-extension install
        exit 0
    fi
    say "Kirie $version is already installed. Use --force to reinstall it."
    exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf -- "$tmp"' EXIT
trap 'exit 130' INT TERM

bundle="kirie-$version-$arch-linux"
say "Downloading Kirie $version ($arch)"
fetch "$repo/releases/download/v$version/$bundle.tar.gz" "$tmp/$bundle.tar.gz" ||
    die "cannot download Kirie $version for $arch. Nothing was installed."
expected="$(expected_sha256 "$tmp" "$bundle")"
[ -n "$expected" ] || die "$bundle.tar.gz is missing from the checksums. Nothing was installed."
[ "$(sha256 "$tmp/$bundle.tar.gz")" = "$expected" ] ||
    die "the download does not match its checksum. Nothing was installed."
tar -xzf "$tmp/$bundle.tar.gz" -C "$tmp"
src="$tmp/$bundle"
[ -x "$src/kirie" ] || die "the archive has no kirie binary. Nothing was installed."

# Written next to the old binary and renamed over it, so a running Kirie keeps working.
mkdir -p "$bin_dir"
cp "$src/kirie" "$bin_dir/.kirie.new"
chmod 755 "$bin_dir/.kirie.new"
mv -f "$bin_dir/.kirie.new" "$binary"

# KWin only lets an app take screenshots when its desktop entry names the binary by full path.
exec_path="$(desktop_exec "$binary")"
mkdir -p "$data_dir/applications" "$data_dir/metainfo"
sed "s|^Exec=kirie|Exec=$exec_path|" "$src/packaging/$app_id.desktop" \
    >"$data_dir/applications/$app_id.desktop"
cp "$src/packaging/$app_id.metainfo.xml" "$data_dir/metainfo/"
for icon in $icon_files; do
    mkdir -p "$data_dir/icons/hicolor/${icon%/*}"
    cp "$src/packaging/icons/hicolor/$icon" "$data_dir/icons/hicolor/$icon"
done
refresh_icon_cache

if [ "$want_extension" = 1 ]; then
    "$binary" gnome-extension install
fi

mkdir -p "${marker%/*}"
printf 'method=script\nversion=%s\n' "$version" >"$marker"

if [ -n "$previous" ]; then
    say ""
    if [ "$previous" = "$version" ]; then
        say "Reinstalled Kirie $version."
    else
        say "Updated Kirie $previous → $version."
    fi
    if [ "$want_extension" = 1 ]; then
        say "Log out and back in so GNOME Shell loads the updated extension."
    fi
    exit 0
fi

say ""
say "Installed Kirie $version to $binary."
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) say "Add $bin_dir to your PATH to run \`kirie\` from a shell." ;;
esac
say "Next: run \`kirie shortcuts install\` to take screenshots with the Print key."
if [ "$want_extension" = 1 ]; then
    say "Then log out and back in once, so GNOME Shell loads Kirie's extension."
fi
