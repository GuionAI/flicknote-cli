use async_trait::async_trait;

use crate::error::CliError;
use crate::types::{Note, Project};

// ─── Filter / request types ──────────────────────────────────────────────────

pub struct NoteFilter<'a> {
    pub project_id: Option<&'a str>,
    pub note_type: Option<&'a str>,
    pub archived: bool,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataFilter {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSearch {
    pub keywords: Vec<String>,
    pub extractions: Vec<MetadataFilter>,
}

pub struct InsertNoteReq<'a> {
    pub id: &'a str,
    pub note_type: &'a str,
    pub status: &'a str,
    pub title: Option<&'a str>,
    pub content: Option<&'a str>,
    pub metadata: Option<&'a str>,
    pub project_id: Option<&'a str>,
    pub now: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertedNote {
    pub uuid: String,
    pub short_id: Option<i64>,
}

pub(crate) enum NoteLookup<'a> {
    ShortId(i64),
    Uuid(&'a str),
}

// ─── Shared helpers ──────────────────────────────────────────────────────────

pub(crate) fn parse_note_lookup(input: &str) -> Result<NoteLookup<'_>, CliError> {
    if input.chars().all(|c| c.is_ascii_digit()) {
        let short_id = input.parse::<i64>().map_err(|_| CliError::NoteNotFound {
            id: input.to_string(),
        })?;
        return Ok(NoteLookup::ShortId(short_id));
    }
    if uuid::Uuid::parse_str(input).is_ok() {
        return Ok(NoteLookup::Uuid(input));
    }
    Err(CliError::NoteNotFound {
        id: input.to_string(),
    })
}

// ─── NoteDb trait ────────────────────────────────────────────────────────────

#[async_trait]
pub trait NoteDb: Send + Sync {
    fn user_id(&self) -> &str;

    // Note resolution
    async fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError>;
    async fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError>;

    // Note reads
    async fn find_note(&self, id: &str) -> Result<Note, CliError>;
    async fn find_archived_note(&self, id: &str) -> Result<Note, CliError>;
    async fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError>;
    async fn list_notes(&self, filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError>;
    async fn search_notes(
        &self,
        keywords: &[String],
        filter: &NoteFilter<'_>,
    ) -> Result<Vec<Note>, CliError>;
    async fn search_notes_structured(
        &self,
        search: &NoteSearch,
        filter: &NoteFilter<'_>,
    ) -> Result<Vec<Note>, CliError>;

    // Note writes
    async fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<InsertedNote, CliError>;
    /// Update content. When `requeue` is true, also sets status = 'ai_queued'.
    async fn update_note_content(
        &self,
        id: &str,
        content: &str,
        requeue: bool,
    ) -> Result<(), CliError>;
    /// Set deleted_at to the given timestamp, or NULL when `deleted_at` is None.
    /// `now` is used for the `updated_at` column and must match the timestamp
    /// used in the hook payload so subscribers see consistent values.
    async fn set_note_deleted_at(
        &self,
        id: &str,
        deleted_at: Option<&str>,
        now: &str,
    ) -> Result<(), CliError>;

    /// Restore the most recently deleted note (sets deleted_at = NULL).
    /// Returns `Ok(())` for both "note restored" and "nothing to undo" — callers
    /// cannot distinguish the two cases.
    async fn undo_last_delete(&self) -> Result<(), CliError>;

    // Project reads
    async fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError>;
    async fn find_project_name_by_id(&self, project_id: &str) -> Result<Option<String>, CliError>;
    async fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError>;
    async fn find_project(&self, id: &str) -> Result<Project, CliError>;
    async fn resolve_project_id(&self, prefix: &str) -> Result<String, CliError>;

    // Project writes
    async fn create_project(&self, name: &str) -> Result<String, CliError>;

    /// Move a note to a different project. Returns the deleted project name if the old
    /// project is now empty. Returns `NoteNotFound` if no such note exists.
    async fn move_note_to_project(
        &self,
        note_id: &str,
        new_project_id: &str,
        old_project_id: Option<&str>,
    ) -> Result<Option<String>, CliError>;

    /// Update project color. `None` = don't change, `Some(None)` = clear, `Some(Some(v))` = set.
    async fn update_project(&self, id: &str, color: Option<Option<&str>>) -> Result<(), CliError>;

    /// Delete (archive) a project by ID. Returns `ProjectNotFound` if no such project exists.
    async fn delete_project(&self, id: &str) -> Result<(), CliError>;

    // Note metadata writes
    /// Update a note's title. Returns `NoteNotFound` if no such note exists.
    async fn update_note_title(&self, id: &str, title: &str) -> Result<(), CliError>;
    /// Update a note's flagged status. Returns `NoteNotFound` if no such note exists.
    async fn update_note_flagged(&self, id: &str, flagged: bool) -> Result<(), CliError>;

    // Note reads (extended)
    async fn count_notes(&self, filter: &NoteFilter<'_>) -> Result<u64, CliError>;
    async fn list_note_topics(
        &self,
        note_ids: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<String>>, CliError>;
    /// Read extraction rows for one or more notes. Returns a map of note_id -> Vec<(key, value)>.
    /// `extraction_keys` filters which keys to read (e.g. `::topic`, `::company`).
    /// Results are ordered by key then value for deterministic rendering.
    async fn list_note_extractions(
        &self,
        note_ids: &[&str],
        extraction_keys: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>, CliError>;
    async fn list_extraction_values(
        &self,
        extraction_keys: &[&str],
        archived: bool,
    ) -> Result<Vec<String>, CliError>;
    /// Replace all extraction rows for one note and one managed key in a single operation.
    /// `values` replaces all rows of the given key for the note.
    /// An empty vec clears all rows for that key.
    async fn set_note_extractions(
        &self,
        note_id: &str,
        extraction_key: &str,
        values: &[String],
    ) -> Result<(), CliError>;
}

#[cfg(feature = "powersync")]
mod local;
#[cfg(feature = "powersync")]
pub use local::LocalPowerSyncBackend;

#[cfg(test)]
#[cfg(feature = "powersync")]
mod tests;
