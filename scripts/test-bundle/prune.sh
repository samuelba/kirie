#!/bin/sh
# Uninstalls Kirie and removes everything it leaves behind, so the next test starts from a clean
# account: its configuration, cache and state, the GNOME shortcut, the extension's settings and
# the screenshot permission the portal remembers. Screenshots you saved stay.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
uuid="kirie@samuelba.dev"
bin_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}"
cache_dir="${XDG_CACHE_HOME:-$HOME/.cache}"
state_dir="${XDG_STATE_HOME:-$HOME/.local/state}"
media_keys="org.gnome.settings-daemon.plugins.media-keys"
keybinding="/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/kirie/"

say() { printf '%s\n' "$*"; }
has() { command -v "$1" >/dev/null 2>&1; }

# Removes ITEM from the string list SCHEMA KEY, when the schema exists and lists it.
drop_from_list() { # SCHEMA KEY ITEM
    list="$(gsettings get "$1" "$2" 2>/dev/null)" || return 0
    case "$list" in
        *"'$3'"*) ;;
        *) return 0 ;;
    esac
    updated="$(printf '%s\n' "$list" | sed -e "s|'$3', ||g" -e "s|, '$3'||g" -e "s|'$3'||g")"
    gsettings set "$1" "$2" "$updated"
    say "Removed $3 from $1 $2."
}

if [ -e "$bin_dir/kirie" ] || [ -e "$data_dir/kirie/install-method" ]; then
    sh "$here/installer.sh" --uninstall
fi

for dir in "$data_dir/gnome-shell/extensions/$uuid" "$config_dir/kirie" "$cache_dir/kirie" \
    "$state_dir/kirie" "$data_dir/kirie" "${XDG_RUNTIME_DIR:+$XDG_RUNTIME_DIR/kirie}"; do
    if [ -n "$dir" ] && [ -e "$dir" ]; then
        rm -rf -- "$dir"
        say "Removed $dir"
    fi
done

if has gsettings; then
    drop_from_list "$media_keys" custom-keybindings "$keybinding"
    drop_from_list org.gnome.shell enabled-extensions "$uuid"
    drop_from_list org.gnome.shell disabled-extensions "$uuid"
fi
if has dconf; then
    dconf reset -f "$keybinding"
    dconf reset -f /org/gnome/shell/extensions/kirie/
elif has gsettings; then
    gsettings reset-recursively "$media_keys.custom-keybinding:$keybinding" 2>/dev/null || true
fi

# The screenshot portal remembers its answer per app; programs started from a terminal share
# the empty app ID, so this also forgets the answer for other unsandboxed programs.
if has gdbus; then
    for app in dev.samuelba.Kirie ""; do
        gdbus call --session --dest org.freedesktop.impl.portal.PermissionStore \
            --object-path /org/freedesktop/impl/portal/PermissionStore \
            --method org.freedesktop.impl.portal.PermissionStore.DeletePermission \
            screenshot screenshot "$app" >/dev/null 2>&1 || true
    done
    say "Reset the screenshot portal's permission."
fi

if grep -qs kirie "$config_dir/kglobalshortcutsrc"; then
    say "KDE still has a Kirie shortcut: remove it in System Settings → Keyboard → Shortcuts."
fi
if grep -qs kirie "$config_dir/cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom"; then
    say "COSMIC still has a Kirie shortcut: remove it in Settings → Keyboard → Custom Shortcuts."
fi
say "Pruned Kirie. Log out and back in if GNOME Shell had loaded its extension."
