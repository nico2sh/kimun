use std::path::PathBuf;

use clap::{Parser, Subcommand, arg, command};
use kimun_core::NoteVault;
use kimun_rag::KimunRag;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Index,
    Ask {
        #[arg(short, long)]
        query: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::new()
        .filter(Some("kimun_"), log::LevelFilter::max())
        .init();
    let cli = Cli::parse();

    let mut rag = KimunRag::sqlite(&std::env::current_dir()?);

    match cli.command {
        Commands::Index => {
            rag.init()?;
            let vault_path = PathBuf::from("/Users/nhormazabal/OneDrive/Notes");
            let vault = NoteVault::new(vault_path)?;
            rag.store_embeddings(vault).await?;
        }
        Commands::Ask { query } => {
            let answer = rag.query(query).await?;

            println!("{answer}");
        }
    }

    Ok(())
}
