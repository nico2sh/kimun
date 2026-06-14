//! `kimun update [--check]` — check for a newer release and, where the install
//! channel permits, self-update in place.

use color_eyre::eyre::Result;

use crate::update;

/// Run the update command. `check_only` reports without downloading.
pub async fn run(check_only: bool) -> Result<()> {
    let config_dir = crate::settings::config_dir()?;

    // The update check is blocking (ureq) — run it off the async runtime.
    let status = {
        let dir = config_dir.clone();
        tokio::task::spawn_blocking(move || update::check(&dir, true)).await??
    };

    // With force = true the check always queries GitHub, so `None` only means a
    // genuine absence of any stable release.
    let Some(status) = status else {
        println!("Could not determine the latest version.");
        return Ok(());
    };

    println!("Current version: {}", status.current);
    println!("Latest version:  {}", status.latest);

    if !status.update_available {
        println!("kimün is up to date.");
        return Ok(());
    }

    println!("Update available: {} → {}", status.current, status.latest);

    if check_only {
        return Ok(());
    }

    if !status.channel.self_update_eligible() {
        match status.channel.upgrade_hint() {
            Some(cmd) => println!("To upgrade, run: {cmd}"),
            None => println!(
                "This install cannot self-update. Download the latest release from \
                 https://github.com/nico2sh/kimun/releases"
            ),
        }
        return Ok(());
    }

    println!("Downloading and installing {}...", status.latest);
    let latest = tokio::task::spawn_blocking(update::fetch_latest).await??;
    tokio::task::spawn_blocking(move || update::apply(&latest)).await??;
    println!(
        "Updated to {}. Restart kimün to use the new version.",
        status.latest
    );
    Ok(())
}
