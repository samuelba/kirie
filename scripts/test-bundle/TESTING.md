# Testing a Kirie build

```sh
./install.sh                    # or ./install.sh --gnome-extension
./uninstall.sh                  # what a user gets; ~/.config/kirie stays
./prune.sh                      # back to a clean account for the next run
```

Kirie needs a Wayland session: `echo $XDG_SESSION_TYPE` must print `wayland`. Kubuntu 24.04
starts Plasma on X11 by default, so pick **Plasma (Wayland)** on the login screen.

## Every desktop

- [ ] `kirie --version` prints the version; the app menu shows Kirie with its icon.
- [ ] `kirie capture` shows the overlay on every screen; select, annotate, copy, save.
- [ ] Opening Kirie from the app menu opens the editor; *Capture a Region* works from its menu.
- [ ] `kirie shortcuts install`, then the printed shortcut takes a screenshot.
- [ ] `./uninstall.sh` removes `~/.local/bin/kirie` and the menu entry.

## GNOME (Ubuntu 24.04)

- [ ] Without the extension, captures go through the screenshot portal (it may ask once).
- [ ] `kirie shortcuts install` adds Shift+Print under Settings → Keyboard → Custom Shortcuts.
- [ ] `./install.sh --gnome-extension`, log out and in: `gnome-extensions info kirie@samuelba.dev`
      says ENABLED, and Shift+Print shows the overlay above every window without a dialog.
- [ ] After `./prune.sh` and a fresh login, the extension and the shortcut are gone.

## KDE Plasma (Kubuntu 24.04)

- [ ] `kirie capture` captures through KWin without a dialog (the desktop entry grants it).
- [ ] `kirie shortcuts install` explains how to add the shortcut; doing so works.

## When something fails

- Log: `RUST_LOG=kirie=debug kirie capture 2>&1 | tee kirie.log`
- A blank or crashing window in a VM without 3D acceleration: retry with
  `ICED_BACKEND=tiny-skia`.
