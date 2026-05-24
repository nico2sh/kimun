// tui/src/cli/commands/labels.rs
use crate::cli::output::OutputFormat;
use color_eyre::eyre::Result;
use kimun_core::NoteVault;

pub async fn run(
    vault: &NoteVault,
    format: OutputFormat,
    workspace_name: &str,
) -> Result<()> {
    let counts = vault.label_counts().await?;

    match format {
        OutputFormat::Text => {
            if counts.is_empty() {
                println!("(no labels)");
            } else {
                for (name, count) in &counts {
                    let suffix = if *count == 1 { "note" } else { "notes" };
                    println!("{} ({} {})", name, count, suffix);
                }
            }
        }
        OutputFormat::Paths => {
            for (name, _) in &counts {
                println!("{}", name);
            }
        }
        OutputFormat::Json => {
            let total = counts.len();
            let labels: Vec<serde_json::Value> = counts
                .iter()
                .map(|(name, count)| {
                    serde_json::json!({ "name": name, "note_count": count })
                })
                .collect();
            let out = serde_json::json!({
                "workspace": workspace_name,
                "total": total,
                "labels": labels,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    // Integration tests live in tui/tests/.
}
