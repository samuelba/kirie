#!/usr/bin/env bash
# Runs Kirie under a headless sway: captures the screen to a file, then shows the overlay and
# captures that, so a reviewer can look at it. Writes into ${1:-target/smoke}.
#
# Needs sway and a built kirie (KIRIE, default target/debug/kirie).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="${1:-$root/target/smoke}"
kirie="${KIRIE:-$root/target/debug/kirie}"
mkdir -p "$out"

runtime="$(mktemp -d)"
chmod 700 "$runtime"
config="$runtime/sway.conf"
cat >"$config" <<'CONF'
output HEADLESS-1 resolution 1280x720 position 0 0
output HEADLESS-2 resolution 1920x1080 position 1280 0 scale 1.5
CONF

export XDG_RUNTIME_DIR="$runtime"
export WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman WLR_HEADLESS_OUTPUTS=2
export ICED_BACKEND=tiny-skia
unset DISPLAY
sway -c "$config" >"$out/sway.log" 2>&1 &
sway_pid=$!
trap 'kill "$sway_pid" 2>/dev/null || true; rm -rf -- "$runtime"' EXIT

for _ in $(seq 50); do
    socket="$(find "$runtime" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -print -quit)"
    [[ -n "$socket" ]] && break
    sleep 0.1
done
[[ -n "${socket:-}" ]] || { cat "$out/sway.log"; echo "sway did not start" >&2; exit 1; }
export WAYLAND_DISPLAY="${socket##*/}"

"$kirie" debug globals | head -3
"$kirie" capture all --output "$out/screen.png"

"$kirie" capture >"$out/overlay.log" 2>&1 &
overlay_pid=$!
sleep 3
"$kirie" capture all --output "$out/overlay.png"
kill "$overlay_pid" 2>/dev/null || true
wait "$overlay_pid" 2>/dev/null || true
echo "Wrote $out/screen.png and $out/overlay.png"
