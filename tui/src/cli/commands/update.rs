//! `kimun update [--check]` — check for a newer release and, where the install
//! channel permits, self-update in place.

use color_eyre::eyre::Result;

use crate::update;

/// Run the update command. `check_only` reports without downloading.
pub async fn run(check_only: bool) -> Result<()> {
    let config_dir = crate::settings::config_dir()?;

    // One GitHub round-trip: fetch the release once, derive the status from it,
    // and (if applying) reuse the very same release — no second fetch.
    let latest = update::latest_release().await?;
    let status = update::status_for(&config_dir, &latest);

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
                "This install cannot self-update. Download the latest release from {}",
                update::releases_url()
            ),
        }
        return Ok(());
    }

    println!("Downloading and installing {}...", status.latest);
    update::install(latest).await?;
    println!(
        "Updated to {}. Restart kimün to use the new version.",
        status.latest
    );
    Ok(())
}
