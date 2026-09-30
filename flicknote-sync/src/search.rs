//! Lexical note search seam shared by local FTS and remote PGroonga adapters.
use flicknote_core::services::dto::{NoteFindInput, SearchHit};

#[async_trait::async_trait]
pub trait NoteSearch: Send + Sync {
    async fn find(&self, input: &NoteFindInput) -> Result<Vec<SearchHit>, String>;
}
