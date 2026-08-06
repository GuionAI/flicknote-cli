//! Note application service.

use crate::backend::{MetadataFilter, NoteDb, NoteFilter, NoteSearch};
use crate::{ENTITY_EXTRACTION_KEYS, TOPIC_EXTRACTION_KEY};

use super::dto::NoteAddInput;
use super::dto::{
    ExtractionDto, NoteArchiveResult, NoteDetail, NoteMutationResult, NoteSectionResult,
    NoteSummary, OpenResult, SectionDto, ShareResult, UnshareResult,
};
pub use super::dto::{
    ExtractionFilterDto, InsertPosition, NoteCountInput, NoteFindInput, NoteListInput,
    NoteModifyInput,
};
use super::edit_match;
use super::editable_document;
use super::error::ServiceError;
use super::markdown;
use super::note_content::extract_title_and_strip;
use super::ports::{BrowserOpener, CreateNote, NoteCreator, ShareGateway, ShareResource};
use super::sections::{content_starts_with_heading, find_section};
use super::source::{SourceResult, SourceView, parse_source};

pub struct NoteService<'a> {
    db: &'a dyn NoteDb,
}

impl<'a> NoteService<'a> {
    pub fn new(db: &'a dyn NoteDb) -> Self {
        Self { db }
    }

    pub async fn list(&self, input: NoteListInput) -> Result<Vec<NoteSummary>, ServiceError> {
        let project_id = match input.project.as_deref() {
            Some(name) => Some(
                self.db
                    .find_project_by_name(name)
                    .await?
                    .ok_or_else(|| ServiceError::ProjectNotFound(name.to_string()))?,
            ),
            None => None,
        };
        let notes = self
            .db
            .list_notes(&NoteFilter {
                project_id: project_id.as_deref(),
                note_type: input.note_type.as_deref(),
                archived: input.archived,
                limit: input.limit,
            })
            .await?;
        let mut summaries = Vec::with_capacity(notes.len());
        for note in notes {
            summaries.push(self.summary(note).await?);
        }
        Ok(summaries)
    }

    pub async fn find(&self, input: NoteFindInput) -> Result<Vec<NoteSummary>, ServiceError> {
        if input.keywords.is_empty() && input.extractions.is_empty() {
            return Err(ServiceError::InvalidArgument(
                "at least one keyword or extraction filter is required".to_string(),
            ));
        }
        let project_id = self
            .resolve_project_filter(input.project.as_deref())
            .await?;
        let search = NoteSearch {
            keywords: input.keywords,
            extractions: input
                .extractions
                .into_iter()
                .map(|filter| MetadataFilter {
                    key: filter.key,
                    value: filter.value,
                })
                .collect(),
        };
        let notes = self
            .db
            .search_notes_structured(
                &search,
                &NoteFilter {
                    project_id: project_id.as_deref(),
                    note_type: None,
                    archived: input.archived,
                    limit: input.limit,
                },
            )
            .await?;
        let mut summaries = Vec::with_capacity(notes.len());
        for note in notes {
            summaries.push(self.summary(note).await?);
        }
        Ok(summaries)
    }

    pub async fn count(&self, input: NoteCountInput) -> Result<u64, ServiceError> {
        let project_id = self
            .resolve_project_filter(input.project.as_deref())
            .await?;
        let filter = NoteFilter {
            project_id: project_id.as_deref(),
            note_type: input.note_type.as_deref(),
            archived: input.archived,
            limit: u32::MAX,
        };
        if input.keywords.is_empty() {
            return Ok(self.db.count_notes(&filter).await?);
        }
        Ok(self.db.search_notes(&input.keywords, &filter).await?.len() as u64)
    }

    pub async fn add(
        &self,
        creator: &dyn NoteCreator,
        input: NoteAddInput,
    ) -> Result<NoteSummary, ServiceError> {
        if input.content.trim().is_empty() {
            return Err(ServiceError::InvalidArgument(
                "content must not be empty".to_string(),
            ));
        }
        let project_id = self
            .resolve_project_filter(input.project.as_deref())
            .await?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let link_url = input.content.trim();
        let is_url = input.interpret_as_url
            && (link_url.starts_with("http://") || link_url.starts_with("https://"))
            && !link_url.chars().any(char::is_whitespace);
        let request = if is_url {
            CreateNote {
                id,
                note_type: "link".to_string(),
                status: "source_queued".to_string(),
                title: None,
                content: None,
                metadata: Some(serde_json::json!({ "link": { "url": link_url } }).to_string()),
                project_id,
                now,
                topics: Vec::new(),
            }
        } else {
            let (title, content) = extract_title_and_strip(&input.content);
            CreateNote {
                id,
                note_type: "normal".to_string(),
                status: "ai_queued".to_string(),
                title,
                content: Some(content),
                metadata: None,
                project_id,
                now,
                topics: Vec::new(),
            }
        };
        let inserted = creator.create(request).await?;
        let note = self.db.find_note(&inserted.uuid).await?;
        self.summary(note).await
    }

    pub async fn get(&self, note_id: &str, archived: bool) -> Result<NoteDetail, ServiceError> {
        let full_id = if archived {
            self.db.resolve_archived_note_id(note_id).await?
        } else {
            self.db.resolve_note_id(note_id).await?
        };
        let note = if archived {
            self.db.find_archived_note(&full_id).await?
        } else {
            self.db.find_note(&full_id).await?
        };
        let content = editable_document::render_editable_note(self.db, &note).await?;
        let mut extraction_keys = Vec::with_capacity(ENTITY_EXTRACTION_KEYS.len() + 1);
        extraction_keys.push(TOPIC_EXTRACTION_KEY);
        extraction_keys.extend_from_slice(ENTITY_EXTRACTION_KEYS);
        let extractions = self
            .db
            .list_note_extractions(&[&full_id], &extraction_keys)
            .await?
            .remove(&full_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(key, value)| ExtractionDto { key, value })
            .collect();
        let metadata = note
            .metadata
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| ServiceError::Internal(error.to_string()))?;
        let sections = markdown::parse_markdown(note.content.as_deref().unwrap_or(""))
            .build_tree()
            .into_iter()
            .map(SectionDto::from)
            .collect();
        Ok(NoteDetail {
            note: self.summary(note).await?,
            content,
            metadata,
            extractions,
            sections,
        })
    }

    pub async fn get_section(
        &self,
        note_id: &str,
        section: &str,
    ) -> Result<NoteSectionResult, ServiceError> {
        let full_id = self.db.resolve_note_id(note_id).await?;
        let note = self.db.find_note(&full_id).await?;
        let content = note.content.as_deref().ok_or(ServiceError::NoTextContent)?;
        let document = markdown::parse_markdown(content);
        let bounds = find_section(&document, section, &full_id)?;
        Ok(NoteSectionResult {
            id: bounds.heading.id.clone(),
            level: bounds.heading.level,
            title: bounds.heading.text.clone(),
            content: content[bounds.start..bounds.end].trim().to_string(),
        })
    }

    pub async fn source(
        &self,
        note_id: &str,
        archived: bool,
        view: SourceView,
        range: Option<&str>,
    ) -> Result<SourceResult, ServiceError> {
        let full_id = if archived {
            self.db.resolve_archived_note_id(note_id).await?
        } else {
            self.db.resolve_note_id(note_id).await?
        };
        let note = if archived {
            self.db.find_archived_note(&full_id).await?
        } else {
            self.db.find_note(&full_id).await?
        };
        let source = note.source.as_deref().ok_or(ServiceError::NoSource)?;
        parse_source(source, view, range)
    }

    pub async fn share(
        &self,
        gateway: &dyn ShareGateway,
        note_id: &str,
    ) -> Result<ShareResult, ServiceError> {
        let id = self.db.resolve_note_id(note_id).await?;
        let url = gateway.share(ShareResource::Note, &id).await?;
        Ok(ShareResult { url })
    }

    pub async fn unshare(
        &self,
        gateway: &dyn ShareGateway,
        note_id: &str,
    ) -> Result<UnshareResult, ServiceError> {
        let id = self.db.resolve_note_id(note_id).await?;
        gateway.unshare(ShareResource::Note, &id).await?;
        Ok(UnshareResult { revoked: true })
    }

    pub async fn open(
        &self,
        opener: &dyn BrowserOpener,
        web_url: &str,
        note_id: &str,
    ) -> Result<OpenResult, ServiceError> {
        if web_url.trim().is_empty() {
            return Err(ServiceError::ConfigMissing("webUrl".to_string()));
        }
        let id = self.db.resolve_note_id(note_id).await?;
        let note = self.db.find_note(&id).await?;
        let url_id = note
            .short_id
            .map(|short_id| short_id.to_string())
            .unwrap_or(id);
        let url = format!("{}/notes/{url_id}", web_url.trim_end_matches('/'));
        opener.open(&url)?;
        Ok(OpenResult { url, opened: true })
    }

    pub async fn append(
        &self,
        note_id: &str,
        content: &str,
    ) -> Result<NoteMutationResult, ServiceError> {
        if content.trim().is_empty() {
            return Err(ServiceError::InvalidArgument(
                "content must not be empty".to_string(),
            ));
        }
        let full_id = self.db.resolve_note_id(note_id).await?;
        let existing = self.db.find_note_content(&full_id).await?;
        let combined = match existing.as_deref() {
            Some(existing) if !existing.is_empty() => format!("{existing}\n\n{content}"),
            _ => content.to_string(),
        };
        self.db
            .update_note_content(&full_id, &combined, false)
            .await?;
        self.mutation_result(&full_id, &combined).await
    }

    pub async fn replace_section(
        &self,
        note_id: &str,
        section: &str,
        replacement: &str,
    ) -> Result<NoteMutationResult, ServiceError> {
        if !content_starts_with_heading(replacement) {
            return Err(ServiceError::InvalidArgument(
                "replacement content must start with a Markdown heading".to_string(),
            ));
        }
        let full_id = self.db.resolve_note_id(note_id).await?;
        let content = self.required_content(&full_id).await?;
        let document = markdown::parse_markdown(&content);
        let bounds = find_section(&document, section, &full_id)?;
        let shifted = markdown::cap_heading_level(replacement.trim(), bounds.heading.level);
        let updated =
            markdown::replace_entire_section(&content, bounds.start, bounds.end, &shifted);
        let updated = updated.trim();
        self.db.update_note_content(&full_id, updated, true).await?;
        self.mutation_result(&full_id, updated).await
    }

    pub async fn rename_section(
        &self,
        note_id: &str,
        section: &str,
        name: &str,
    ) -> Result<NoteMutationResult, ServiceError> {
        if name.trim().is_empty() {
            return Err(ServiceError::InvalidArgument(
                "section name must not be empty".to_string(),
            ));
        }
        let full_id = self.db.resolve_note_id(note_id).await?;
        let content = self.required_content(&full_id).await?;
        let document = markdown::parse_markdown(&content);
        let bounds = find_section(&document, section, &full_id)?;
        let heading_line_end = content[bounds.start..]
            .find('\n')
            .map(|offset| bounds.start + offset)
            .unwrap_or(content.len());
        let updated = format!(
            "{}{} {}{}",
            &content[..bounds.start],
            "#".repeat(bounds.heading.level),
            name.trim(),
            &content[heading_line_end..]
        );
        let updated = updated.trim();
        self.db.update_note_content(&full_id, updated, true).await?;
        self.mutation_result(&full_id, updated).await
    }

    pub async fn insert(
        &self,
        note_id: &str,
        section: &str,
        position: InsertPosition,
        insertion: &str,
    ) -> Result<NoteMutationResult, ServiceError> {
        if insertion.trim().is_empty() {
            return Err(ServiceError::InvalidArgument(
                "content must not be empty".to_string(),
            ));
        }
        let full_id = self.db.resolve_note_id(note_id).await?;
        let content = self.required_content(&full_id).await?;
        let document = markdown::parse_markdown(&content);
        let bounds = find_section(&document, section, &full_id)?;
        let split = match position {
            InsertPosition::Before => bounds.start,
            InsertPosition::After => bounds.end,
        };
        let before = content[..split].trim_end_matches('\n');
        let after = content[split..].trim_start_matches('\n');
        let insertion = insertion.trim_end();
        let updated = if before.is_empty() {
            format!("{insertion}\n\n{after}")
        } else if after.is_empty() {
            format!("{before}\n\n{insertion}")
        } else {
            format!("{before}\n\n{insertion}\n\n{after}")
        };
        let updated = updated.trim();
        self.db.update_note_content(&full_id, updated, true).await?;
        self.mutation_result(&full_id, updated).await
    }

    pub async fn delete_section(
        &self,
        note_id: &str,
        section: &str,
    ) -> Result<NoteMutationResult, ServiceError> {
        let full_id = self.db.resolve_note_id(note_id).await?;
        let content = self.required_content(&full_id).await?;
        let document = markdown::parse_markdown(&content);
        let bounds = find_section(&document, section, &full_id)?;
        let before = &content[..bounds.start];
        let after = &content[bounds.end..];
        let updated = format!(
            "{}{}",
            before.trim_end_matches('\n'),
            if after.is_empty() {
                String::new()
            } else {
                format!("\n\n{}", after.trim_start_matches('\n'))
            }
        );
        let updated = updated.trim();
        self.db.update_note_content(&full_id, updated, true).await?;
        self.mutation_result(&full_id, updated).await
    }

    pub async fn modify(&self, input: NoteModifyInput) -> Result<NoteMutationResult, ServiceError> {
        let has_edit = input.before.is_some() || input.after.is_some();
        if input.before.is_some() != input.after.is_some() {
            return Err(ServiceError::InvalidArgument(
                "before and after must be provided together".to_string(),
            ));
        }
        if input.section.is_some() && !has_edit {
            return Err(ServiceError::InvalidArgument(
                "section requires before and after".to_string(),
            ));
        }
        if !has_edit && input.project.is_none() && input.flagged.is_none() {
            return Err(ServiceError::NothingToModify);
        }

        let full_id = self.db.resolve_note_id(&input.id).await?;
        let note = self.db.find_note(&full_id).await?;
        let resolved_project = match input.project.as_deref() {
            Some(name) => Some(
                self.db
                    .find_project_by_name(name)
                    .await?
                    .ok_or_else(|| ServiceError::ProjectNotFound(name.to_string()))?,
            ),
            None => None,
        };

        let mut resulting_content = note.content.clone().unwrap_or_default();
        if let (Some(before), Some(after)) = (input.before.as_deref(), input.after.as_deref()) {
            if let Some(section) = input.section.as_deref() {
                let content = self.required_content(&full_id).await?;
                let document = markdown::parse_markdown(&content);
                let bounds = find_section(&document, section, &full_id)?;
                let scoped = &content[bounds.start..bounds.end];
                let matched = edit_match::find_unique(scoped, before)?;
                let absolute = edit_match::MatchInfo {
                    start: bounds.start + matched.start,
                    end: bounds.start + matched.end,
                };
                resulting_content = edit_match::splice(&content, &absolute, after);
                self.db
                    .update_note_content(&full_id, resulting_content.trim(), true)
                    .await?;
            } else {
                let editable = editable_document::render_editable_note(self.db, &note).await?;
                let matched = edit_match::find_unique(&editable, before)?;
                let updated = edit_match::splice(&editable, &matched, after);
                resulting_content =
                    editable_document::save_editable_note(self.db, &full_id, &updated)
                        .await?
                        .stored_content;
            }
        }

        if let Some(project_id) = resolved_project
            && note.project_id.as_deref() != Some(project_id.as_str())
        {
            self.db
                .move_note_to_project(&full_id, &project_id, note.project_id.as_deref())
                .await?;
        }
        if let Some(flagged) = input.flagged {
            self.db.update_note_flagged(&full_id, flagged).await?;
        }

        self.mutation_result(&full_id, &resulting_content).await
    }

    pub async fn archive(&self, note_id: &str) -> Result<NoteArchiveResult, ServiceError> {
        let full_id = self.db.resolve_note_id(note_id).await?;
        let note = self.db.find_note(&full_id).await?;
        let now = chrono::Utc::now().to_rfc3339();
        self.db
            .set_note_deleted_at(&full_id, Some(&now), &now)
            .await?;
        Ok(NoteArchiveResult {
            short_id: note.short_id,
            uuid: note.id,
            archived: true,
        })
    }

    pub async fn restore(&self, note_id: &str) -> Result<NoteArchiveResult, ServiceError> {
        let full_id = self.db.resolve_archived_note_id(note_id).await?;
        let note = self.db.find_archived_note(&full_id).await?;
        let now = chrono::Utc::now().to_rfc3339();
        self.db.set_note_deleted_at(&full_id, None, &now).await?;
        Ok(NoteArchiveResult {
            short_id: note.short_id,
            uuid: note.id,
            archived: false,
        })
    }

    async fn required_content(&self, note_id: &str) -> Result<String, ServiceError> {
        self.db
            .find_note_content(note_id)
            .await?
            .ok_or(ServiceError::NoTextContent)
    }

    async fn resolve_project_filter(
        &self,
        project: Option<&str>,
    ) -> Result<Option<String>, ServiceError> {
        match project {
            Some(name) => {
                Ok(Some(self.db.find_project_by_name(name).await?.ok_or_else(
                    || ServiceError::ProjectNotFound(name.to_string()),
                )?))
            }
            None => Ok(None),
        }
    }

    async fn mutation_result(
        &self,
        note_id: &str,
        content: &str,
    ) -> Result<NoteMutationResult, ServiceError> {
        let note = self.db.find_note(note_id).await?;
        let summary = self.summary(note).await?;
        let sections = markdown::parse_markdown(content)
            .build_tree()
            .into_iter()
            .map(SectionDto::from)
            .collect();
        Ok(NoteMutationResult {
            note: summary,
            sections,
        })
    }

    async fn summary(&self, note: crate::types::Note) -> Result<NoteSummary, ServiceError> {
        let project = match note.project_id.as_deref() {
            Some(project_id) => self.db.find_project_name_by_id(project_id).await?,
            None => None,
        };
        let topics = self
            .db
            .list_note_topics(&[&note.id])
            .await?
            .remove(&note.id)
            .unwrap_or_default();
        Ok(NoteSummary {
            short_id: note.short_id,
            uuid: note.id,
            note_type: note.r#type,
            status: note.status,
            title: note.title,
            project_id: note.project_id,
            project,
            topics,
            summary: note.summary,
            flagged: note.is_flagged == Some(1),
            created_at: note.created_at,
            updated_at: note.updated_at,
            deleted_at: note.deleted_at,
        })
    }
}

#[cfg(all(test, feature = "powersync"))]
mod tests {
    use std::cell::RefCell;

    use crate::backend::NoteDb;
    use crate::services::dto::NoteAddInput;
    use crate::services::ports::{
        BrowserOpener, CreateNote, NoteCreator, ShareGateway, ShareResource,
    };
    use crate::services::test_support::{insert_normal_note, make_backend};
    use async_trait::async_trait;

    use super::{
        ExtractionFilterDto, InsertPosition, NoteCountInput, NoteFindInput, NoteListInput,
        NoteModifyInput, NoteService,
    };

    #[tokio::test]
    async fn append_separates_content_and_does_not_requeue() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "existing", "synced").await;
        let service = NoteService::new(&backend);

        let result = service.append(&id, "added").await.unwrap();

        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.content.as_deref(), Some("existing\n\nadded"));
        assert_eq!(note.status, "synced");
        assert_eq!(result.note.uuid, id);
        assert!(result.sections.is_empty());
    }

    #[tokio::test]
    async fn replace_section_replaces_subtree_and_requeues() {
        let backend = make_backend().await;
        let id = insert_normal_note(
            &backend,
            "## Target\nold\n\n### Child\nchild\n\n## Keep\nstable",
            "synced",
        )
        .await;
        let section = crate::services::markdown::parse_markdown(
            "## Target\nold\n\n### Child\nchild\n\n## Keep\nstable",
        )
        .headings[0]
            .id
            .clone();
        let service = NoteService::new(&backend);

        let result = service
            .replace_section(&id, &section, "# Replacement\nnew")
            .await
            .unwrap();

        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(
            note.content.as_deref(),
            Some("## Replacement\nnew\n\n## Keep\nstable")
        );
        assert_eq!(note.status, "ai_queued");
        assert_eq!(result.sections[0].title, "Replacement");
    }

    #[tokio::test]
    async fn modify_rejects_ambiguous_before_without_writing() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "same\n\nsame", "synced").await;
        let service = NoteService::new(&backend);

        let error = service
            .modify(NoteModifyInput {
                id: id.clone(),
                before: Some("same".to_string()),
                after: Some("changed".to_string()),
                section: None,
                project: None,
                flagged: None,
            })
            .await
            .unwrap_err();

        assert_eq!(error.code(), "before_ambiguous");
        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.content.as_deref(), Some("same\n\nsame"));
        assert_eq!(note.status, "synced");
    }

    #[tokio::test]
    async fn archive_and_restore_target_the_explicit_note() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "body", "synced").await;
        let service = NoteService::new(&backend);

        let archived = service.archive(&id).await.unwrap();
        assert!(archived.archived);
        assert!(backend.find_note(&id).await.is_err());

        let restored = service.restore(&id).await.unwrap();
        assert!(!restored.archived);
        assert_eq!(backend.find_note(&id).await.unwrap().id, id);
    }

    #[tokio::test]
    async fn list_returns_typed_summary_with_project_name() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "body", "synced").await;
        let project_id = backend.create_project("work").await.unwrap();
        backend
            .move_note_to_project(&id, &project_id, None)
            .await
            .unwrap();
        let service = NoteService::new(&backend);

        let notes = service
            .list(NoteListInput {
                note_type: None,
                project: Some("work".to_string()),
                archived: false,
                limit: 20,
            })
            .await
            .unwrap();

        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].uuid, id);
        assert_eq!(notes[0].project.as_deref(), Some("work"));
    }

    #[tokio::test]
    async fn get_returns_editable_content_and_section_tree() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "## Part\nBody", "synced").await;
        let service = NoteService::new(&backend);

        let detail = service.get(&id, false).await.unwrap();

        assert!(detail.content.contains("title: Test note"));
        assert_eq!(detail.sections[0].title, "Part");
        assert_eq!(detail.note.uuid, id);
    }

    #[tokio::test]
    async fn rename_and_delete_section_update_the_same_tree_contract() {
        let backend = make_backend().await;
        let original = "## First\none\n\n## Second\ntwo";
        let id = insert_normal_note(&backend, original, "synced").await;
        let section = crate::services::markdown::parse_markdown(original).headings[0]
            .id
            .clone();
        let service = NoteService::new(&backend);

        let renamed = service
            .rename_section(&id, &section, "Renamed")
            .await
            .unwrap();
        assert_eq!(renamed.sections[0].title, "Renamed");
        let renamed_id = renamed.sections[0].id.clone();

        let deleted = service.delete_section(&id, &renamed_id).await.unwrap();
        assert_eq!(deleted.sections.len(), 1);
        assert_eq!(deleted.sections[0].title, "Second");
    }

    #[tokio::test]
    async fn insert_after_section_places_content_after_the_whole_subtree() {
        let backend = make_backend().await;
        let original = "## First\none\n\n### Child\nchild\n\n## Second\ntwo";
        let id = insert_normal_note(&backend, original, "synced").await;
        let section = crate::services::markdown::parse_markdown(original).headings[0]
            .id
            .clone();
        let service = NoteService::new(&backend);

        service
            .insert(&id, &section, InsertPosition::After, "## New\nnew")
            .await
            .unwrap();

        let content = backend.find_note_content(&id).await.unwrap().unwrap();
        assert_eq!(
            content,
            "## First\none\n\n### Child\nchild\n\n## New\nnew\n\n## Second\ntwo"
        );
    }

    #[tokio::test]
    async fn find_and_count_use_typed_filters() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "PowerSync notes", "synced").await;
        backend
            .set_note_extractions(&id, "::topic", &["Rust".to_string()])
            .await
            .unwrap();
        let service = NoteService::new(&backend);

        let found = service
            .find(NoteFindInput {
                keywords: Vec::new(),
                extractions: vec![ExtractionFilterDto {
                    key: "::topic".to_string(),
                    value: "Rust".to_string(),
                }],
                project: None,
                archived: false,
                limit: 20,
            })
            .await
            .unwrap();
        assert_eq!(found[0].uuid, id);

        let count = service
            .count(NoteCountInput {
                keywords: vec!["PowerSync".to_string()],
                project: None,
                note_type: None,
                archived: false,
            })
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn get_section_returns_heading_and_full_subtree() {
        let backend = make_backend().await;
        let original = "## First\none\n\n### Child\nchild\n\n## Second\ntwo";
        let id = insert_normal_note(&backend, original, "synced").await;
        let section = crate::services::markdown::parse_markdown(original).headings[0]
            .id
            .clone();
        let service = NoteService::new(&backend);

        let result = service.get_section(&id, &section).await.unwrap();

        assert_eq!(result.id, section);
        assert_eq!(result.title, "First");
        assert_eq!(result.content, "## First\none\n\n### Child\nchild");
    }

    #[tokio::test]
    async fn get_section_accepts_an_id_returned_by_get_when_title_matches_heading() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "# Test note\nBody", "synced").await;
        let service = NoteService::new(&backend);
        let detail = service.get(&id, false).await.unwrap();
        let section_id = detail.sections[0].id.clone();

        let section = service.get_section(&id, &section_id).await.unwrap();

        assert_eq!(section.id, section_id);
        assert_eq!(section.title, "Test note");
        assert_eq!(section.content, "# Test note\nBody");
    }

    struct DbCreator<'a> {
        db: &'a dyn NoteDb,
        request: RefCell<Option<CreateNote>>,
    }

    #[async_trait(?Send)]
    impl NoteCreator for DbCreator<'_> {
        async fn create(
            &self,
            request: CreateNote,
        ) -> Result<crate::backend::InsertedNote, crate::services::error::ServiceError> {
            let inserted = self.db.insert_note(&request.as_insert_request()).await?;
            self.request.replace(Some(request));
            Ok(inserted)
        }
    }

    #[tokio::test]
    async fn add_normalizes_h1_before_calling_creator() {
        let backend = make_backend().await;
        let creator = DbCreator {
            db: &backend,
            request: RefCell::new(None),
        };
        let service = NoteService::new(&backend);

        let created = service
            .add(
                &creator,
                NoteAddInput {
                    content: "# Title\n\nBody".to_string(),
                    project: None,
                    interpret_as_url: true,
                },
            )
            .await
            .unwrap();

        let request = creator.request.borrow();
        let request = request.as_ref().unwrap();
        assert_eq!(request.note_type, "normal");
        assert_eq!(request.title.as_deref(), Some("Title"));
        assert_eq!(request.content.as_deref(), Some("Body"));
        assert_eq!(created.title.as_deref(), Some("Title"));
    }

    #[tokio::test]
    async fn add_only_treats_a_pure_http_value_as_a_link() {
        let backend = make_backend().await;
        let creator = DbCreator {
            db: &backend,
            request: RefCell::new(None),
        };

        NoteService::new(&backend)
            .add(
                &creator,
                NoteAddInput {
                    content: "https://example.com with context".to_string(),
                    project: None,
                    interpret_as_url: true,
                },
            )
            .await
            .unwrap();

        let request = creator.request.borrow();
        let request = request.as_ref().unwrap();
        assert_eq!(request.note_type, "normal");
        assert_eq!(
            request.content.as_deref(),
            Some("https://example.com with context")
        );
    }

    #[tokio::test]
    async fn source_reads_archived_notes_through_the_shared_parser() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "body", "synced").await;
        sqlx::query("UPDATE notes SET source = ? WHERE id = ?")
            .bind(r#"{"link":{"content":"one\ntwo"}}"#)
            .bind(&id)
            .execute(&backend.db.pool)
            .await
            .unwrap();
        let service = NoteService::new(&backend);
        service.archive(&id).await.unwrap();

        let result = service
            .source(
                &id,
                true,
                crate::services::source::SourceView::Rendered,
                Some("2"),
            )
            .await
            .unwrap();

        let crate::services::source::SourceResult::Rendered { content, .. } = result else {
            panic!("expected rendered source");
        };
        assert_eq!(content, "two\n");
    }

    #[derive(Default)]
    struct FakeSideEffects {
        shared: RefCell<Vec<(ShareResource, String)>>,
        opened: RefCell<Vec<String>>,
    }

    #[async_trait(?Send)]
    impl ShareGateway for FakeSideEffects {
        async fn share(
            &self,
            resource: ShareResource,
            id: &str,
        ) -> Result<String, crate::services::error::ServiceError> {
            self.shared.borrow_mut().push((resource, id.to_string()));
            Ok(format!("https://share.example/{id}"))
        }

        async fn unshare(
            &self,
            resource: ShareResource,
            id: &str,
        ) -> Result<(), crate::services::error::ServiceError> {
            self.shared.borrow_mut().push((resource, id.to_string()));
            Ok(())
        }
    }

    impl BrowserOpener for FakeSideEffects {
        fn open(&self, url: &str) -> Result<(), crate::services::error::ServiceError> {
            self.opened.borrow_mut().push(url.to_string());
            Ok(())
        }
    }

    #[tokio::test]
    async fn share_and_open_resolve_note_identity_before_side_effects() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "body", "synced").await;
        let side_effects = FakeSideEffects::default();
        let service = NoteService::new(&backend);

        let shared = service.share(&side_effects, &id).await.unwrap();
        assert_eq!(shared.url, format!("https://share.example/{id}"));
        assert_eq!(
            side_effects.shared.borrow().as_slice(),
            &[(ShareResource::Note, id.clone())]
        );

        let unshared = service.unshare(&side_effects, &id).await.unwrap();
        assert!(unshared.revoked);

        let opened = service
            .open(&side_effects, "https://app.example/", &id)
            .await
            .unwrap();
        assert_eq!(opened.url, format!("https://app.example/notes/{id}"));
        assert!(opened.opened);
        assert_eq!(side_effects.opened.borrow().as_slice(), &[opened.url]);
    }
}
