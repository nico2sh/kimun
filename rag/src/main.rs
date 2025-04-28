use std::path::PathBuf;

use clap::{Parser, Subcommand};
use db::VecDB;
use document::ChunkLoader;
use embedder::{Embedder, fastembedder::FastEmbedder};
use kimun_core::NoteVault;
use llmclients::{LLMClient, mistral::MistralClient};

mod db;
mod document;
mod embedder;
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
        .filter(Some("rag_"), log::LevelFilter::max())
        .init();
    let cli = Cli::parse();

    let db = VecDB::new();

    let embedder = FastEmbedder::new()?;

    match cli.command {
        Commands::Index => {
            db.init()?;
            let vault_path = PathBuf::from("/Users/nhormazabal/OneDrive/Notes");
            let vault = NoteVault::new(vault_path)?;
            let chunk_loader = ChunkLoader::new(vault);
            let chunks = chunk_loader.load_notes()?;

            let embeddings = embedder.generate_embeddings(&chunks).await?;

            let embed_chunks = embeddings.chunks(100);
            let mut i = 0;
            for batch in embed_chunks {
                let mut insert_batch = vec![];
                for c in batch {
                    insert_batch.push((i, chunks.get(i).unwrap(), c));
                    i += 1;
                }
                db.insert_vec(insert_batch)?;
            }
        }
        Commands::Ask { query } => {
            let query_embed = embedder.prompt_embedding(&query).await?;

            let context = db.get_docs(&query_embed)?;

            let mistral = MistralClient::new();
            let answer = mistral.ask(query, context).await?;
            println!("{answer}");
        }
    }

    Ok(())
}
