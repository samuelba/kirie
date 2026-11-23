//! `kirie gnome-extension install|uninstall`: the companion GNOME Shell extension, embedded in
//! the binary so it always matches this version of Kirie.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const UUID: &str = "kirie@samuelba.dev";
/// The extension's metadata, which names the release and so changes with every one.
pub const METADATA: &str = "metadata.json";
const SCHEMA_ID: &str = "org.gnome.shell.extensions.kirie";

const FILES: [(&str, &str); 4] = [
    (
        METADATA,
        include_str!("../../../gnome-extension/metadata.json"),
    ),
    (
        "extension.js",
        include_str!("../../../gnome-extension/extension.js"),
    ),
    ("LICENSE", include_str!("../../../gnome-extension/LICENSE")),
    (
        "schemas/org.gnome.shell.extensions.kirie.gschema.xml",
        include_str!(
            "../../../gnome-extension/schemas/org.gnome.shell.extensions.kirie.gschema.xml"
        ),
    ),
];

/// Where the extension is installed for this user, as `install.sh` puts it too.
pub fn dir() -> Option<PathBuf> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_dir().join("gnome-shell/extensions").join(UUID))
}

fn target() -> anyhow::Result<PathBuf> {
    dir().ok_or_else(|| anyhow::anyhow!("cannot find the data folder (HOME is not set)"))
}

pub fn install() -> anyhow::Result<()> {
    let target = target()?;
    write_files(&target)
        .map_err(|error| anyhow::anyhow!("cannot write {}: {error}", target.display()))?;
    let schemas = target.join("schemas");
    run("glib-compile-schemas", &[schemas.as_os_str()]).map_err(anyhow::Error::msg)?;
    println!(
        "Installed the GNOME Shell extension to {}",
        target.display()
    );
    point_extension_at_this_binary(&schemas);
    match run("gnome-extensions", &["enable".as_ref(), UUID.as_ref()]) {
        Ok(()) => println!("Enabled it."),
        Err(_) => println!("Enable it with: gnome-extensions enable {UUID}"),
    }
    println!(
        "Log out and back in once so GNOME Shell loads it. Then Shift+Print takes a screenshot."
    );
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let target = target()?;
    let _ = run("gnome-extensions", &["disable".as_ref(), UUID.as_ref()]);
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
        println!("Removed {}", target.display());
    }
    Ok(())
}

fn write_files(target: &Path) -> std::io::Result<()> {
    for (name, content) in FILES {
        let path = target.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
    }
    Ok(())
}

/// GNOME Shell does not see the user's shell PATH (e.g. ~/.local/bin), so the extension is told
/// exactly which binary to start.
fn point_extension_at_this_binary(schemas: &Path) {
    let Ok(binary) = crate::launch::exe() else {
        return;
    };
    let value = format!("[{}]", quote(&binary));
    let args = [
        "--schemadir".as_ref(),
        schemas.as_os_str(),
        "set".as_ref(),
        SCHEMA_ID.as_ref(),
        "command".as_ref(),
        value.as_ref(),
    ];
    match run("gsettings", &args) {
        Ok(()) => println!("The extension starts {}", binary.display()),
        Err(error) => eprintln!("kirie: could not tell the extension where Kirie is: {error}"),
    }
}

/// A `GVariant` string literal for a path.
fn quote(path: &Path) -> String {
    let escaped = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('\'', "\\'");
    format!("'{escaped}'")
}

fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_extension_files_are_written() {
        let target =
            std::env::temp_dir().join(format!("kirie-extension-test-{}", std::process::id()));
        write_files(&target).expect("write files");
        for (name, content) in FILES {
            assert_eq!(
                std::fs::read_to_string(target.join(name)).expect(name),
                content
            );
        }
        std::fs::remove_dir_all(&target).expect("clean up");
    }

    #[test]
    fn metadata_names_the_uuid_and_schema() {
        let metadata = FILES[0].1;
        assert!(metadata.contains(UUID));
        assert!(metadata.contains(SCHEMA_ID));
    }

    #[test]
    fn paths_are_quoted_for_gsettings() {
        assert_eq!(quote(Path::new("/usr/bin/kirie")), "'/usr/bin/kirie'");
        assert_eq!(
            quote(Path::new("/home/o'neil/bin/kirie")),
            "'/home/o\\'neil/bin/kirie'"
        );
    }
}
