//! `kirie self-update`: installs the latest release after checking that its checksums are signed
//! by the release key, the same way the install script does when minisign is installed.

use crate::release::{self, InstallKind};

pub fn run() -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(release::install_kind(), InstallKind::Script { .. }),
        "Kirie was not installed with the install script; update it the way it was installed"
    );
    let Some(latest) = release::check_blocking().map_err(anyhow::Error::msg)? else {
        println!("Kirie {} is the latest version.", release::CURRENT_VERSION);
        return Ok(());
    };
    println!(
        "Updating Kirie {} → {}",
        release::CURRENT_VERSION,
        latest.version
    );
    // The installer tells how it went, and whether to log out for the GNOME Shell extension.
    release::install_blocking(&latest.version, true).map_err(anyhow::Error::msg)?;
    Ok(())
}
