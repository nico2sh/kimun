use std::path::PathBuf;

use clap::{Parser, Subcommand};
use document::ChunkLoader;

use embeddings::Embeddings;
use embeddings::vecsqlite::VecSQLite;
use kimun_core::NoteVault;
use llmclients::{LLMClient, mistral::MistralClient};

mod document;
mod embeddings;
mod llmclients;

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

    let mut db = VecSQLite::new();

    match cli.command {
        Commands::Index => {
            db.init()?;
            let vault_path = PathBuf::from("/Users/nhormazabal/OneDrive/Notes");
            let vault = NoteVault::new(vault_path)?;
            let chunk_loader = ChunkLoader::new(vault);
            let chunks = chunk_loader.load_notes()?;

            db.store_embeddings(&chunks).await?;
        }
        Commands::Ask { query } => {
            let context = db.query_embedding(&query).await?;

            let mistral = MistralClient::new();
            let answer = mistral.ask(query, context).await?;
            println!("{answer}");
        }
    }

    Ok(())
}
