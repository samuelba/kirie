//! `kirie shortcuts install`: a keyboard shortcut that takes a screenshot, set where the desktop
//! allows it and explained where it does not.

mod accelerator;
mod cosmic;

use std::process::Command;

use accelerator::Accelerator;

/// Print is the desktop's own screenshot key on GNOME and COSMIC; Kirie takes Shift+Print.
const DEFAULT_KEYS: &str = "<Shift>Print";
const GNOME_PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/kirie/";
const GNOME_LIST: &str = "org.gnome.settings-daemon.plugins.media-keys";
const GNOME_ENTRY: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Desktop {
    Cosmic,
    Gnome,
    Kde,
    Other,
}

fn desktop(current: &str) -> Desktop {
    let names: Vec<String> = current
        .split(':')
        .map(|name| name.trim().to_ascii_uppercase())
        .collect();
    let has = |name: &str| names.iter().any(|entry| entry == name);
    if has("COSMIC") {
        Desktop::Cosmic
    } else if has("GNOME") || has("GNOME-CLASSIC") {
        Desktop::Gnome
    } else if has("KDE") {
        Desktop::Kde
    } else {
        Desktop::Other
    }
}

pub fn install() -> anyhow::Result<()> {
    let exe = crate::launch::exe()?;
    let command = format!("{} capture", exe.display());
    let keys = Accelerator::parse(DEFAULT_KEYS).map_err(anyhow::Error::msg)?;
    match desktop(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()) {
        Desktop::Cosmic => match cosmic::bind(keys, &command).map_err(anyhow::Error::msg)? {
            cosmic::Change::Unchanged => println!("COSMIC already runs Kirie on {keys}."),
            cosmic::Change::Added | cosmic::Change::Updated => {
                println!("Added {keys} to COSMIC's keyboard shortcuts.");
            }
        },
        Desktop::Gnome => gnome(&command, keys)?,
        Desktop::Kde => {
            println!(
                "In System Settings → Keyboard → Shortcuts, choose Add New → Command or Script,"
            );
            println!("enter `{command}` and give it {keys}.");
        }
        Desktop::Other => {
            println!("Bind a key in your compositor's config, for example:");
            println!("  sway / i3:   bindsym Shift+Print exec {command}");
            println!("  Hyprland:    bind = SHIFT, Print, exec, {command}");
            println!(
                "  niri:        Shift+Print {{ spawn \"{}\" \"capture\"; }}",
                exe.display()
            );
        }
    }
    Ok(())
}

/// The extension brings its own shortcut; without it, a GNOME custom shortcut runs Kirie.
fn gnome(command: &str, keys: Accelerator) -> anyhow::Result<()> {
    let extension = crate::extension::dir();
    if extension.is_some_and(|dir| dir.exists()) {
        println!("Kirie's GNOME Shell extension already takes screenshots on {keys}.");
        return Ok(());
    }
    let list = gsettings(&["get", GNOME_LIST, "custom-keybindings"])?;
    if let Some(updated) = with_path(&list, GNOME_PATH) {
        gsettings(&["set", GNOME_LIST, "custom-keybindings", &updated])?;
    }
    let entry = format!("{GNOME_ENTRY}:{GNOME_PATH}");
    gsettings(&["set", &entry, "name", "'Kirie'"])?;
    let quoted = format!("'{}'", command.replace('\\', "\\\\").replace('\'', "\\'"));
    gsettings(&["set", &entry, "command", &quoted])?;
    gsettings(&["set", &entry, "binding", &format!("'{keys}'")])?;
    println!("Added {keys} to GNOME's custom shortcuts.");
    println!(
        "For an overlay above every window and no permission dialog, also run: kirie gnome-extension install"
    );
    Ok(())
}

/// The `GVariant` string list `list` with `path` added, or `None` when it is already there.
fn with_path(list: &str, path: &str) -> Option<String> {
    let list = list.trim().trim_start_matches("@as ");
    let quoted = format!("'{path}'");
    if list.contains(&quoted) {
        return None;
    }
    let inner = list.trim_start_matches('[').trim_end_matches(']').trim();
    Some(if inner.is_empty() {
        format!("[{quoted}]")
    } else {
        format!("[{inner}, {quoted}]")
    })
}

fn gsettings(args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("gsettings").args(args).output()?;
    anyhow::ensure!(
        output.status.success(),
        "gsettings {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktops_are_recognized() {
        assert_eq!(desktop("COSMIC"), Desktop::Cosmic);
        assert_eq!(desktop("ubuntu:GNOME"), Desktop::Gnome);
        assert_eq!(desktop("GNOME-Classic:GNOME"), Desktop::Gnome);
        assert_eq!(desktop("KDE"), Desktop::Kde);
        assert_eq!(desktop("sway"), Desktop::Other);
        assert_eq!(desktop(""), Desktop::Other);
    }

    #[test]
    fn the_gnome_list_gains_kirie_once() {
        assert_eq!(with_path("@as []", "/k/"), Some("['/k/']".to_owned()));
        assert_eq!(
            with_path("['/a/']\n", "/k/"),
            Some("['/a/', '/k/']".to_owned())
        );
        assert_eq!(with_path("['/a/', '/k/']", "/k/"), None);
    }

    #[test]
    fn the_default_keys_parse() {
        assert!(Accelerator::parse(DEFAULT_KEYS).is_ok());
    }
}
