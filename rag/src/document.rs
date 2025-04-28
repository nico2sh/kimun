// A document is a chunk of text for Kimun
// Several can be in a single file as the document is basically a section within a file
// split by Markdown titles
//

use kimun_core::{NoteVault, nfs::VaultPath, note::NoteDetails};

pub struct KimunChunk {
    pub content: String,
    pub metadata: KimunMetadata,
}

impl KimunChunk {
    pub fn to_embed_payload(&self) -> String {
        format!("passage: {}\n{}", self.metadata.title, self.content)
    }
}

pub struct KimunMetadata {
    pub source_path: String,
    pub title: String,
    pub date: Option<chrono::NaiveDate>,
}

impl KimunMetadata {
    pub fn get_date_string(&self) -> Option<String> {
        self.date.map(|d| d.format("%Y-%m-%d").to_string())
    }
}

pub struct ChunkLoader {
    vault: NoteVault,
}

impl ChunkLoader {
    pub fn new(vault: NoteVault) -> Self {
        Self { vault }
    }

    pub fn load_notes(&self) -> anyhow::Result<Vec<KimunChunk>> {
        let mut result = Vec::new();
        // let path_chunks = self.vault.get_note_chunks(&VaultPath::new("journal"))?;
        let path_chunks = self.vault.get_note_chunks(&VaultPath::new("journal"))?;
        for (path, chunks) in path_chunks.iter() {
            let (_parent, file) = path.get_parent_path();
            let filename = file.strip_suffix(".md").unwrap_or(file.as_str());

            let date = match chrono::NaiveDate::parse_from_str(filename, "%Y-%m-%d") {
                Ok(d) => Some(d),
                Err(_) => None,
            };
            for chunk in chunks {
                let title = if chunk.breadcrumb.is_empty() {
                    NoteDetails::get_title_from_text(&chunk.text)
                } else {
                    chunk.breadcrumb.join(", ")
                };
                let metadata = KimunMetadata {
                    source_path: path.to_string(),
                    title,
                    date,
                };
                let document = KimunChunk {
                    content: chunk.text.clone(),
                    metadata,
                };
                result.push(document);
            }
        }

        Ok(result)
    }
}
