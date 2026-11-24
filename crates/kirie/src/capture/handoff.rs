use std::path::Path;

use anyhow::Context as _;

use crate::desktop::{ipc, launch};
use crate::runtime::block_on;

/// Opens a project from the hand-off folder in the running editor, or in a new one.
pub(super) fn open(path: &Path) -> anyhow::Result<()> {
    if !block_on(ipc::hand_over(Some(path), None))? {
        editor_command(&launch::exe()?, path)
            .spawn()
            .context("cannot open the editor")?;
    }
    Ok(())
}

/// The editor, for a picture the overlay hands over. The extension trusts only the overlay it
/// started, so the editor must not pass for it; its captures go through the extension again.
fn editor_command(exe: &Path, path: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command
        .arg("edit")
        .arg(path)
        .env_remove(kirie_capture::shell::LAUNCHED_ENV);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_editor_the_overlay_opens_is_not_trusted_by_the_extension() {
        let command = editor_command(Path::new("kirie"), Path::new("a.kirie"));
        assert!(
            command.get_envs().any(|(key, value)| {
                key == kirie_capture::shell::LAUNCHED_ENV && value.is_none()
            })
        );
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["edit", "a.kirie"]);
    }
}
