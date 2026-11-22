#!/usr/bin/env bash
# Headless smoke test for the GNOME Shell extension. Runs a throwaway GNOME Shell whose D-Bus
# session, dconf database, runtime dir and extension dir all live under target/gnome-smoke, so
# the real session is never touched. Writes screenshots and a log to target/gnome-smoke/out.
#
# Needs gnome-shell and a built kirie (cargo build -p kirie).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dev="$root/target/gnome-smoke"
uuid="kirie@samuelba.dev"
devtools="devtools@kirie.local"

if [[ "${1:-}" != "--session" ]]; then
    rm -rf -- "$dev"
    mkdir -p "$dev/config" "$dev/state" "$dev/cache" "$dev/out" \
        "$dev/data/gnome-shell/extensions/$devtools"
    install -d -m 0700 "$dev/runtime"
    ln -sfn "$root/gnome-extension" "$dev/data/gnome-shell/extensions/$uuid"
    cat >"$dev/data/gnome-shell/extensions/$devtools/metadata.json" <<JSON
{"uuid": "$devtools", "name": "Kirie devtools", "description": "Test-only: unsafe mode.", "shell-version": ["46", "47", "48", "49", "50"]}
JSON
    cat >"$dev/data/gnome-shell/extensions/$devtools/extension.js" <<'JS'
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
export default class DevtoolsExtension extends Extension {
    enable() { global.context.unsafe_mode = true; }
    disable() { global.context.unsafe_mode = false; }
}
JS
    glib-compile-schemas --strict "$root/gnome-extension/schemas"
    export XDG_CONFIG_HOME="$dev/config" XDG_DATA_HOME="$dev/data" XDG_STATE_HOME="$dev/state"
    export XDG_CACHE_HOME="$dev/cache" XDG_RUNTIME_DIR="$dev/runtime"
    exec dbus-run-session -- bash "$0" --session
fi

call() { gdbus call --session "$@"; }
evaljs() { call --dest org.gnome.Shell --object-path /org/gnome/Shell --method org.gnome.Shell.Eval "$1"; }
shot() {
    call --dest org.gnome.Shell --object-path /org/gnome/Shell/Screenshot \
        --method org.gnome.Shell.Screenshot.Screenshot false false "$dev/out/$1.png" >/dev/null
}
windows() {
    evaljs 'JSON.stringify(global.display.list_all_windows().map(w => { const r = w.get_frame_rect();
        return {title: w.get_title(), above: w.is_above(), fullscreen: w.is_fullscreen(),
            skipTaskbar: w.skip_taskbar, monitor: w.get_monitor(), rect: [r.x, r.y, r.width, r.height]}; }))'
}

gsettings set org.gnome.shell disable-user-extensions false
gsettings set org.gnome.shell welcome-dialog-last-shown-version '999'
gsettings set org.gnome.shell enabled-extensions "['$uuid', '$devtools']"
gsettings --schemadir "$root/gnome-extension/schemas" \
    set org.gnome.shell.extensions.kirie command "['$root/target/debug/kirie']"

# Inherited by the Kirie the extension starts.
export KIRIE_OVERLAY_DEMO=1
gnome-shell --headless --virtual-monitor 1280x720 --virtual-monitor 1024x768 --wayland \
    >"$dev/out/shell.log" 2>&1 &
shell_pid=$!
trap 'kill "$shell_pid" 2>/dev/null; wait "$shell_pid" 2>/dev/null' EXIT
for _ in $(seq 1 60); do
    evaljs 'global.context.unsafe_mode' 2>/dev/null | grep -q true && break
    sleep 0.5
done

evaljs 'Main.overview.hide(); "hidden"' >/dev/null
sleep 1

echo "1. the extension owns its name"
call --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.NameHasOwner dev.samuelba.Kirie.Shell

echo "2. strangers may not read the screens"
if call --dest dev.samuelba.Kirie.Shell --object-path /dev/samuelba/Kirie/Shell \
    --method dev.samuelba.Kirie.Shell.Screens 2>"$dev/out/denied.txt"; then
    echo "FAIL: Screens answered a process the extension did not start" >&2
    exit 1
fi
grep -o 'AccessDenied[^)]*' "$dev/out/denied.txt" | head -n 1

echo "2b. callers cannot pass their own arguments"
if call --dest dev.samuelba.Kirie.Shell --object-path /dev/samuelba/Kirie/Shell \
    --method dev.samuelba.Kirie.Shell.Capture "['all', '--output', '/tmp/stolen.png']" 2>/dev/null; then
    echo "FAIL: Capture took free-form arguments" >&2
    exit 1
fi
echo "refused"

echo "3. kirie capture goes through the extension"
"$root/target/debug/kirie" capture
sleep 5
windows | tee "$dev/out/windows.json"
shot overlay
echo "Wrote $dev/out/overlay.png"
