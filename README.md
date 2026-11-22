<p align="center">
  <img src="packaging/icons/hicolor/scalable/apps/dev.samuelba.Kirie.svg" width="96" alt="Kirie">
</p>

# Kirie

Kirie (切り絵, Japanese paper-cut art) is a screenshot and annotation tool for Wayland. Cut a
piece out of your screen, mark it up right there or later in the editor, and copy or save it.

It runs on GNOME, COSMIC, KDE Plasma and the layer-shell compositors (sway, Hyprland, niri,
Wayfire). X11 is not supported.

> Kirie is in early development. There is no release yet.

## Install

```sh
curl -fsSL https://samuelba.dev/kirie/install.sh | sh
```

The script installs Kirie for your user, without sudo, to `~/.local/bin/kirie`, with its desktop
entry and icons. Run it again to update. Uninstall with:

```sh
curl -fsSL https://samuelba.dev/kirie/install.sh | sh -s -- --uninstall
```

When `minisign` or `rsign` is installed, the script checks the release's signature before
installing anything.

## Set up a shortcut

```sh
kirie shortcuts install
```

This binds **Shift+Print** to `kirie capture`, so your desktop's own screenshot tool keeps Print.

| Desktop | How Kirie captures and shows the overlay |
|---|---|
| **GNOME** | Install the companion extension with `kirie gnome-extension install` and log out once. It brings the shortcut, takes screenshots without a permission dialog, and keeps the overlay above every window. Without it, Kirie uses the screenshot portal (GNOME asks for permission once) and a fullscreen window. |
| **COSMIC** | `ext-image-copy-capture` and a layer-shell overlay on every screen. `kirie shortcuts install` adds the shortcut to COSMIC's keyboard settings. |
| **KDE Plasma** | The screenshot portal and a layer-shell overlay. Add the shortcut in System Settings; `kirie shortcuts install` tells you how. |
| **sway, Hyprland, niri, Wayfire** | `wlr-screencopy` or `ext-image-copy-capture` and a layer-shell overlay. Bind `kirie capture` in your compositor's config; `kirie shortcuts install` prints an example. |

## Use it

`kirie capture` freezes every screen. Drag to cut out a region, or click to take a whole screen.
Then mark it up with the toolbar under the selection.

| Key | Does |
|---|---|
| Enter or Ctrl+C | Copy the picture and close |
| Ctrl+S | Save it to `~/Pictures/Screenshots` |
| Ctrl+E | Open it in the editor, with its marks still editable |
| Ctrl+A | Select every screen |
| Arrow keys | Move the selection (with Shift: by 10) |
| Ctrl+Z, Ctrl+Shift+Z | Undo, redo |
| Esc | Put the tool down, then cancel |
| V A L R E P H T N B X D S | Select, Arrow, Line, Rectangle, Ellipse, Pen, Highlighter, Text, Step number, Blur, Pixelate, Redact, Spotlight |

Hold **Shift** while drawing to snap lines to 45° and boxes to squares.

`kirie` opens the editor. Paste an image with Ctrl+V, open one with `kirie edit FILE`, or take a
screenshot from its header. Zoom with Ctrl+scroll, pan with the scroll wheel or the middle
button. Press C, or the crop button under the tools, to keep only part of the image: drag the
frame and press Enter. **Frame** puts room around the picture, filled with a color or left
transparent, with a soft shadow under the picture if you like, and can take in marks drawn
beyond the picture's edges. **Save project** (Ctrl+Shift+S) keeps a `.kirie` file whose marks you can change later.

Saved and copied images hold only the finished picture. Redact covers every pixel it touches
with solid color. Blur and pixelate scramble each block a little, so guesses of what was there
never match exactly, but they keep a trace of light and dark: use Redact for passwords, keys and
anything else that must stay secret. A `.kirie` project keeps the original screenshot, under
every mark, so that the marks stay editable; share the saved image, not the project.

### Styles

- **Clean**: crisp lines with a soft shadow.
- **Sketch**: hand-drawn lines, hatched fills and handwritten text.
- **Ink**: brush strokes that swell and taper, and an open ensō for circles.

Each style has its own shades of the standard colors: bright pastels for Sketch, the pigments of
ink painting for Ink.
Switching styles moves marks in a standard color to the new style's shade and leaves your own
colors as they are.

## Build from source

Kirie needs Rust (stable) and the Wayland and xkbcommon development files.

```sh
sudo dnf install wayland-devel libxkbcommon-devel     # Fedora
sudo apt install libwayland-dev libxkbcommon-dev      # Debian, Ubuntu

cargo build --release
cargo fmt --all --check && cargo clippy --workspace --all-targets && cargo test --workspace
```

| Crate | What it does |
|---|---|
| `kirie-capture` | Captures every screen through the compositor's protocol, Kirie's GNOME Shell extension or the screenshot portal |
| `kirie-annotate` | Documents of marks, the drawing tools, the three styles, rendering and `.kirie` projects |
| `kirie-ui` | Kirie's colors, spacing, icons and controls |
| `kirie` | The command, the on-screen overlay and the editor |

Smoke tests run Kirie under a headless compositor: `scripts/sway-smoke.sh` (needs sway) and
`scripts/gnome-smoke.sh` (needs gnome-shell, runs an isolated session with the extension).

## License

MIT. The GNOME Shell extension in `gnome-extension/` is GPL-2.0-or-later. The bundled fonts
(Inter, Caveat, Source Code Pro) are under the SIL Open Font License, and the icons come from
[Lucide](https://lucide.dev) under the ISC license; their licenses are in `assets/`.
