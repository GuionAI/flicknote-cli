//! Service boundary for the editable full-note Markdown contract.

use crate::frontmatter::{self, EditableDoc};
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;
use flicknote_core::types::Note;

const EXTRACTION_TYPES: [&str; 2] = ["topic", "entity"];
const TOPIC_EXTRACTION: &str = "topic";
const ENTITY_EXTRACTION: &str = "entity";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedEditableNote {
    pub title: String,
    pub stored_content: String,
    pub topics: Vec<String>,
    pub entities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditableSaveResult {
    pub title_changed: bool,
    pub content_changed: bool,
    pub stored_content: String,
}

pub(crate) async fn load_editable_note(db: &dyn NoteDb, note_id: &str) -> Result<String, CliError> {
    let note = db.find_note(note_id).await?;
    render_editable_note(db, &note).await
}

pub(crate) async fn render_editable_note(db: &dyn NoteDb, note: &Note) -> Result<String, CliError> {
    let (topics, entities) = load_managed_extractions(db, &note.id).await?;
    let content = note.content.as_deref().unwrap_or("");
    let (stored_frontmatter, body_without_fm) = frontmatter::split_frontmatter(content);
    Ok(frontmatter::build_editable_content(
        note.title.as_deref(),
        body_without_fm,
        &topics,
        &entities,
        stored_frontmatter,
    ))
}

pub(crate) async fn save_editable_note(
    db: &dyn NoteDb,
    note_id: &str,
    markdown: &str,
) -> Result<EditableSaveResult, CliError> {
    let note = db.find_note(note_id).await?;
    let parsed = parse_editable_note(markdown)?;

    // Only reject missing title when the old note had one — do not allow dropping
    // the title on an existing note. New notes with no title are fine.
    if note.title.as_deref().is_some_and(|t| !t.is_empty()) && parsed.title.trim().is_empty() {
        return Err(CliError::Other(
            "Full-note write requires a non-empty H1 title. \
             This note had a title — removing it is not allowed. \
             Add a `# Title` heading after any frontmatter."
                .into(),
        ));
    }

    let title_changed = note.title.as_deref() != Some(parsed.title.as_str());
    if title_changed {
        db.update_note_title(note_id, &parsed.title).await?;
    }

    db.set_note_extractions(note_id, TOPIC_EXTRACTION, &parsed.topics)
        .await?;
    db.set_note_extractions(note_id, ENTITY_EXTRACTION, &parsed.entities)
        .await?;

    let old_content = note.content.as_deref().unwrap_or("");
    let content_changed = old_content != parsed.stored_content;
    if content_changed {
        db.update_note_content(note_id, &parsed.stored_content, true)
            .await?;
    }

    Ok(EditableSaveResult {
        title_changed,
        content_changed,
        stored_content: parsed.stored_content,
    })
}

pub(crate) fn parse_editable_note(markdown: &str) -> Result<ParsedEditableNote, CliError> {
    let doc = frontmatter::parse_editable_doc(markdown);
    let stored_content = stored_content_from_doc(&doc);

    Ok(ParsedEditableNote {
        title: doc.title.unwrap_or_default(),
        stored_content,
        topics: doc.topics,
        entities: doc.entities,
    })
}

pub(crate) fn normal_note_content_ref(parsed: &ParsedEditableNote) -> Option<&str> {
    Some(parsed.stored_content.as_str())
}

fn stored_content_from_doc(doc: &EditableDoc) -> String {
    if let Some(ref fm) = doc.unmanaged_frontmatter {
        if doc.body.is_empty() {
            return fm.clone();
        }
        return format!("{}\n\n{}", fm, doc.body);
    }
    doc.body.clone()
}

async fn load_managed_extractions(
    db: &dyn NoteDb,
    note_id: &str,
) -> Result<(Vec<String>, Vec<String>), CliError> {
    let extractions = db
        .list_note_extractions(&[note_id], &EXTRACTION_TYPES)
        .await?;
    let pairs = extractions.get(note_id);
    let mut topics = Vec::new();
    let mut entities = Vec::new();

    if let Some(pairs) = pairs {
        for (extraction_type, value) in pairs {
            match extraction_type.as_str() {
                TOPIC_EXTRACTION => topics.push(value.clone()),
                ENTITY_EXTRACTION => entities.push(value.clone()),
                _ => {}
            }
        }
    }

    Ok((topics, entities))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flicknote_core::backend::{InsertNoteReq, NoteFilter};
    use flicknote_core::types::{Keyterm, Project, Prompt};
    use std::cell::RefCell;
    use std::collections::HashMap;

    const NOTE_ID: &str = "01234567-89ab-cdef-0123-456789abcdef";

    struct FakeNoteDb {
        note: RefCell<Note>,
        extractions: RefCell<HashMap<String, Vec<(String, String)>>>,
    }

    impl FakeNoteDb {
        fn new(note: Note, extractions: Vec<(String, String)>) -> Self {
            let mut map = HashMap::new();
            map.insert(note.id.clone(), extractions);
            Self {
                note: RefCell::new(note),
                extractions: RefCell::new(map),
            }
        }

        fn note(&self) -> Note {
            self.note.borrow().clone()
        }

        fn extraction_values(&self, extraction_type: &str) -> Vec<String> {
            self.extractions
                .borrow()
                .get(NOTE_ID)
                .into_iter()
                .flatten()
                .filter(|(kind, _)| kind == extraction_type)
                .map(|(_, value)| value.clone())
                .collect()
        }
    }

    fn note_with(content: Option<&str>, title: Option<&str>) -> Note {
        Note {
            id: NOTE_ID.to_string(),
            user_id: "user".to_string(),
            r#type: "normal".to_string(),
            status: "synced".to_string(),
            title: title.map(str::to_string),
            content: content.map(str::to_string),
            summary: None,
            is_flagged: None,
            project_id: None,
            metadata: None,
            source: None,
            external_id: None,
            created_at: None,
            updated_at: None,
            deleted_at: None,
        }
    }

    #[tokio::test]
    async fn load_editable_note_merges_db_extractions_with_stored_custom_frontmatter() {
        let db = FakeNoteDb::new(
            note_with(Some("---\ncustom: keep\n---\nBody.\n"), Some("Title")),
            vec![
                ("topic".to_string(), "rust".to_string()),
                ("entity".to_string(), "PowerSync".to_string()),
            ],
        );

        let markdown = load_editable_note(&db, NOTE_ID).await.unwrap();

        assert!(markdown.starts_with("---\n"));
        assert!(markdown.contains("topics:"));
        assert!(markdown.contains("- rust"));
        assert!(markdown.contains("entities:"));
        assert!(markdown.contains("- PowerSync"));
        assert!(markdown.contains("custom: keep"));
        assert!(markdown.contains("# Title"));
        assert!(markdown.trim_end().ends_with("Body."));
    }

    #[tokio::test]
    async fn save_editable_note_writes_title_body_custom_frontmatter_and_extractions() {
        let db = FakeNoteDb::new(
            note_with(Some("Old body."), Some("Old Title")),
            vec![("topic".to_string(), "old".to_string())],
        );

        let markdown = "---\ntopics: [rust, async]\nentities:\n  - PowerSync\ncustom:\n  nested: true\n---\n# New Title\n\nNew body.";
        let result = save_editable_note(&db, NOTE_ID, markdown).await.unwrap();

        let note = db.note();
        assert_eq!(note.title.as_deref(), Some("New Title"));
        let content = note.content.expect("content should be stored");
        assert!(content.contains("custom:"));
        assert!(content.contains("nested: true"));
        assert!(content.contains("New body."));
        assert!(!content.contains("topics:"));
        assert!(!content.contains("entities:"));
        assert_eq!(
            db.extraction_values("topic"),
            vec!["rust".to_string(), "async".to_string()]
        );
        assert_eq!(
            db.extraction_values("entity"),
            vec!["PowerSync".to_string()]
        );
        assert!(result.title_changed);
        assert!(result.content_changed);
        assert_eq!(result.stored_content, content);
    }

    #[test]
    fn parse_editable_note_covers_round_trip_contract_cases() {
        struct Case {
            name: &'static str,
            markdown: &'static str,
            title: &'static str,
            stored_content: &'static str,
            topics: &'static [&'static str],
            entities: &'static [&'static str],
        }

        let cases = [
            Case {
                name: "title-only note stays an empty text note",
                markdown: "# Title",
                title: "Title",
                stored_content: "",
                topics: &[],
                entities: &[],
            },
            Case {
                name: "body leading whitespace is preserved",
                markdown: "# Title\n\n  indented first line\n\tTabbed second line",
                title: "Title",
                stored_content: "  indented first line\n\tTabbed second line",
                topics: &[],
                entities: &[],
            },
            Case {
                name: "managed empty lists clear extraction rows",
                markdown: "---\ntopics: []\nentities: []\ncustom: keep\n---\n# Title\n\nBody.",
                title: "Title",
                stored_content: "---\ncustom: keep\n---\n\nBody.",
                topics: &[],
                entities: &[],
            },
        ];

        for case in cases {
            let parsed = parse_editable_note(case.markdown).unwrap_or_else(|err| {
                panic!("{} should parse, got {err}", case.name);
            });

            assert_eq!(parsed.title, case.title, "{}", case.name);
            assert_eq!(parsed.stored_content, case.stored_content, "{}", case.name);
            assert_eq!(
                parsed.topics,
                case.topics
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>(),
                "{}",
                case.name
            );
            assert_eq!(
                parsed.entities,
                case.entities
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>(),
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn normal_note_content_ref_preserves_empty_text_content() {
        let parsed = parse_editable_note("# Title").unwrap();

        assert_eq!(normal_note_content_ref(&parsed), Some(""));
    }

    #[async_trait::async_trait(?Send)]
    impl NoteDb for FakeNoteDb {
        fn user_id(&self) -> &str {
            "user"
        }

        async fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError> {
            Ok(prefix.to_string())
        }

        async fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError> {
            Ok(prefix.to_string())
        }

        async fn find_note(&self, id: &str) -> Result<Note, CliError> {
            let note = self.note.borrow();
            if note.id == id {
                return Ok(note.clone());
            }
            Err(CliError::NoteNotFound { id: id.to_string() })
        }

        async fn find_archived_note(&self, id: &str) -> Result<Note, CliError> {
            self.find_note(id).await
        }

        async fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError> {
            Ok(self.find_note(id).await?.content)
        }

        async fn list_notes(&self, _filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError> {
            unimplemented!()
        }

        async fn search_notes(
            &self,
            _keywords: &[String],
            _filter: &NoteFilter<'_>,
        ) -> Result<Vec<Note>, CliError> {
            unimplemented!()
        }

        async fn insert_note(&self, _req: &InsertNoteReq<'_>) -> Result<(), CliError> {
            unimplemented!()
        }

        async fn update_note_content(
            &self,
            id: &str,
            content: &str,
            _requeue: bool,
        ) -> Result<(), CliError> {
            let mut note = self.note.borrow_mut();
            if note.id != id {
                return Err(CliError::NoteNotFound { id: id.to_string() });
            }
            note.content = Some(content.to_string());
            Ok(())
        }

        async fn set_note_deleted_at(
            &self,
            _id: &str,
            _deleted_at: Option<&str>,
            _now: &str,
        ) -> Result<(), CliError> {
            unimplemented!()
        }

        async fn undo_last_delete(&self) -> Result<(), CliError> {
            unimplemented!()
        }

        async fn find_project_by_name(&self, _name: &str) -> Result<Option<String>, CliError> {
            unimplemented!()
        }

        async fn find_project_name_by_id(
            &self,
            _project_id: &str,
        ) -> Result<Option<String>, CliError> {
            unimplemented!()
        }

        async fn list_projects(&self, _archived: bool) -> Result<Vec<Project>, CliError> {
            unimplemented!()
        }

        async fn find_project(&self, _id: &str) -> Result<Project, CliError> {
            unimplemented!()
        }

        async fn resolve_project_id(&self, _prefix: &str) -> Result<String, CliError> {
            unimplemented!()
        }

        async fn create_project(&self, _name: &str) -> Result<String, CliError> {
            unimplemented!()
        }

        async fn move_note_to_project(
            &self,
            _note_id: &str,
            _new_project_id: &str,
            _old_project_id: Option<&str>,
        ) -> Result<Option<String>, CliError> {
            unimplemented!()
        }

        async fn update_project(
            &self,
            _id: &str,
            _prompt_id: Option<Option<&str>>,
            _keyterm_id: Option<Option<&str>>,
            _color: Option<Option<&str>>,
        ) -> Result<(), CliError> {
            unimplemented!()
        }

        async fn delete_project(&self, _id: &str) -> Result<(), CliError> {
            unimplemented!()
        }

        async fn update_note_title(&self, id: &str, title: &str) -> Result<(), CliError> {
            let mut note = self.note.borrow_mut();
            if note.id != id {
                return Err(CliError::NoteNotFound { id: id.to_string() });
            }
            note.title = Some(title.to_string());
            Ok(())
        }

        async fn update_note_flagged(&self, _id: &str, _flagged: bool) -> Result<(), CliError> {
            unimplemented!()
        }

        async fn count_notes(&self, _filter: &NoteFilter<'_>) -> Result<u64, CliError> {
            unimplemented!()
        }

        async fn list_note_topics(
            &self,
            _note_ids: &[&str],
        ) -> Result<HashMap<String, Vec<String>>, CliError> {
            unimplemented!()
        }

        async fn list_note_extractions(
            &self,
            note_ids: &[&str],
            extraction_types: &[&str],
        ) -> Result<HashMap<String, Vec<(String, String)>>, CliError> {
            let mut result = HashMap::new();
            let store = self.extractions.borrow();
            for note_id in note_ids {
                let Some(rows) = store.get(*note_id) else {
                    continue;
                };
                let filtered = rows
                    .iter()
                    .filter(|(kind, _)| extraction_types.contains(&kind.as_str()))
                    .cloned()
                    .collect::<Vec<_>>();
                result.insert((*note_id).to_string(), filtered);
            }
            Ok(result)
        }

        async fn set_note_extractions(
            &self,
            note_id: &str,
            extraction_type: &str,
            values: &[String],
        ) -> Result<(), CliError> {
            let mut store = self.extractions.borrow_mut();
            let rows = store.entry(note_id.to_string()).or_default();
            rows.retain(|(kind, _)| kind != extraction_type);
            rows.extend(
                values
                    .iter()
                    .map(|value| (extraction_type.to_string(), value.clone())),
            );
            Ok(())
        }

        async fn resolve_prompt_id(&self, _prefix: &str) -> Result<String, CliError> {
            unimplemented!()
        }

        async fn insert_prompt(
            &self,
            _id: &str,
            _title: &str,
            _description: Option<&str>,
            _prompt: &str,
            _now: &str,
        ) -> Result<(), CliError> {
            unimplemented!()
        }

        async fn find_prompt(&self, _id: &str) -> Result<Prompt, CliError> {
            unimplemented!()
        }

        async fn list_prompts(&self) -> Result<Vec<Prompt>, CliError> {
            unimplemented!()
        }

        async fn update_prompt(
            &self,
            _id: &str,
            _title: Option<&str>,
            _description: Option<&str>,
            _prompt: Option<&str>,
        ) -> Result<(), CliError> {
            unimplemented!()
        }

        async fn delete_prompt(&self, _id: &str) -> Result<(), CliError> {
            unimplemented!()
        }

        async fn resolve_keyterm_id(&self, _prefix: &str) -> Result<String, CliError> {
            unimplemented!()
        }

        async fn insert_keyterm(
            &self,
            _id: &str,
            _name: &str,
            _description: Option<&str>,
            _content: Option<&str>,
            _now: &str,
        ) -> Result<(), CliError> {
            unimplemented!()
        }

        async fn find_keyterm(&self, _id: &str) -> Result<Keyterm, CliError> {
            unimplemented!()
        }

        async fn list_keyterms(&self) -> Result<Vec<Keyterm>, CliError> {
            unimplemented!()
        }

        async fn update_keyterm(
            &self,
            _id: &str,
            _name: Option<&str>,
            _description: Option<&str>,
            _content: Option<&str>,
        ) -> Result<(), CliError> {
            unimplemented!()
        }

        async fn delete_keyterm(&self, _id: &str) -> Result<(), CliError> {
            unimplemented!()
        }
    }
}
