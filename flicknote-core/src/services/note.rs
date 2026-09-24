//! Note application service.

use crate::backend::{MetadataFilter, NoteDb, NoteFilter, NoteSearch, RouteProjectUpdate};
use crate::{ENTITY_EXTRACTION_KEYS, TOPIC_EXTRACTION_KEY};

use super::dto::{
    ExtractionDto, NoteAddInput, NoteArchiveResult, NoteCreateResult, NoteDetail, NoteListItem,
    NoteMutationResult, NoteRouteProjectInput, NoteRouteProjectResult, NoteSectionResult,
    NoteSummary, OpenResult, Patch, RecallCandidate, SectionDto, ShareResult, UnshareResult,
};
pub use super::dto::{
    ExtractionFilterDto, InsertPosition, NoteCountInput, NoteFindInput, NoteListInput,
    NoteModifyInput,
};

pub const RECALL_MAX_CANDIDATES: u32 = 5;
use super::edit_match;
use super::error::ServiceError;
use super::markdown;
use super::note_content::extract_title_and_strip;
use super::ports::{BrowserOpener, CreateNote, NoteCreator, ShareGateway, ShareResource};
use super::sections::{content_starts_with_heading, find_section};
use super::source::{SourceResult, SourceView, parse_source};

fn validate_created_range(
    created_after: Option<&str>,
    created_before: Option<&str>,
) -> Result<(), ServiceError> {
    let parse = |name: &str, value: &str| {
        chrono::DateTime::parse_from_rfc3339(value).map_err(|_| {
            ServiceError::InvalidArgument(format!("{name} must be an RFC3339 timestamp"))
        })
    };
    let after = created_after
        .map(|value| parse("created_after", value))
        .transpose()?;
    let before = created_before
        .map(|value| parse("created_before", value))
        .transpose()?;
    if after
        .zip(before)
        .is_some_and(|(after, before)| after >= before)
    {
        return Err(ServiceError::InvalidArgument(
            "created_after must be earlier than created_before".to_string(),
        ));
    }
    Ok(())
}

pub struct NoteService<'a> {
    db: &'a dyn NoteDb,
}

impl<'a> NoteService<'a> {
    pub fn new(db: &'a dyn NoteDb) -> Self {
        Self { db }
    }

    pub async fn route_project(
        &self,
        input: Vec<NoteRouteProjectInput>,
    ) -> Result<NoteRouteProjectResult, ServiceError> {
        if input.is_empty() {
            return Err(ServiceError::InvalidArgument(
                "routing batch must not be empty".to_string(),
            ));
        }
        let mut note_ids = std::collections::HashSet::with_capacity(input.len());
        let mut updates = Vec::with_capacity(input.len());
        for item in input {
            if item.note_id <= 0 {
                return Err(ServiceError::InvalidArgument(
                    "note_id must be a positive short ID".to_string(),
                ));
            }
            if !note_ids.insert(item.note_id) {
                return Err(ServiceError::InvalidArgument(format!(
                    "duplicate note_id in routing batch: {}",
                    item.note_id
                )));
            }
            if !item.probability.is_finite() || !(0.0..=1.0).contains(&item.probability) {
                return Err(ServiceError::InvalidArgument(format!(
                    "probability for note {} must be between 0 and 1",
                    item.note_id
                )));
            }
            if let Some(project_id) = item.project_id.as_deref()
                && uuid::Uuid::parse_str(project_id).is_err()
            {
                return Err(ServiceError::InvalidArgument(format!(
                    "project_id for note {} must be a UUID or null",
                    item.note_id
                )));
            }
            updates.push(RouteProjectUpdate {
                note_id: item.note_id,
                project_id: item.project_id,
                probability_json: serde_json::to_string(&item.probability)
                    .map_err(crate::error::CliError::Json)?,
            });
        }
        self.db.route_notes_to_projects(&updates).await?;
        Ok(NoteRouteProjectResult {
            routed: updates.len(),
        })
    }

    pub async fn list(&self, input: NoteListInput) -> Result<Vec<NoteListItem>, ServiceError> {
        if input.cursor.is_some_and(|cursor| cursor <= 0) {
            return Err(ServiceError::InvalidArgument(
                "cursor must be a positive note ID".to_string(),
            ));
        }
        if input.project.is_some() && input.no_project {
            return Err(ServiceError::InvalidArgument(
                "project and no_project are mutually exclusive".to_string(),
            ));
        }
        validate_created_range(
            input.created_after.as_deref(),
            input.created_before.as_deref(),
        )?;
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
                no_project: input.no_project,
                note_type: input.note_type.as_deref(),
                created_after: input.created_after.as_deref(),
                created_before: input.created_before.as_deref(),
                archived: input.archived,
                limit: input.limit,
                cursor: input.cursor,
            })
            .await?;
        let mut items = Vec::with_capacity(notes.len());
        for note in notes {
            items.push(self.list_item(note).await?);
        }
        Ok(items)
    }

    pub async fn find(&self, input: NoteFindInput) -> Result<Vec<NoteListItem>, ServiceError> {
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
                    no_project: false,
                    note_type: None,
                    created_after: None,
                    created_before: None,
                    archived: input.archived,
                    limit: input.limit,
                    cursor: None,
                },
            )
            .await?;
        let mut items = Vec::with_capacity(notes.len());
        for note in notes {
            items.push(self.list_item(note).await?);
        }
        Ok(items)
    }

    /// Hydrate ranked search identifiers from canonical storage. A projection
    /// can lag canonical deletion; missing hits are deliberately skipped.
    pub async fn find_ranked_ids(&self, ids: &[String]) -> Result<Vec<NoteListItem>, ServiceError> {
        let mut items = Vec::with_capacity(ids.len());
        for id in ids {
            match self.db.find_note(id).await {
                Ok(note) => items.push(self.list_item(note).await?),
                Err(crate::error::CliError::NoteNotFound { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(items)
    }

    pub async fn recall(
        &self,
        prompt: &str,
        project: Option<&str>,
    ) -> Result<Vec<RecallCandidate>, ServiceError> {
        if prompt.trim().is_empty() {
            return Ok(Vec::new());
        }
        let project_id = self.resolve_project_filter(project).await?;
        self.db
            .recall_notes(
                prompt,
                &NoteFilter {
                    project_id: project_id.as_deref(),
                    no_project: false,
                    note_type: None,
                    created_after: None,
                    created_before: None,
                    archived: false,
                    limit: RECALL_MAX_CANDIDATES,
                    cursor: None,
                },
            )
            .await
            .map_err(ServiceError::from)
    }

    pub async fn count(&self, input: NoteCountInput) -> Result<u64, ServiceError> {
        let project_id = self
            .resolve_project_filter(input.project.as_deref())
            .await?;
        let filter = NoteFilter {
            project_id: project_id.as_deref(),
            no_project: false,
            note_type: input.note_type.as_deref(),
            created_after: None,
            created_before: None,
            archived: input.archived,
            limit: u32::MAX,
            cursor: None,
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
        let now = input
            .created_at
            .clone()
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        let link_url = input.content.trim();
        let is_url = !input.draft
            && input.interpret_as_url
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
                topics: input.topics.clone(),
                attachment_path: None,
            }
        } else {
            let (title, content) = extract_title_and_strip(&input.content);
            if input.draft && content.trim().is_empty() {
                return Err(ServiceError::InvalidArgument(
                    "content must not be empty".to_string(),
                ));
            }
            CreateNote {
                id,
                note_type: "normal".to_string(),
                status: if input.draft {
                    "draft".to_string()
                } else {
                    "ai_queued".to_string()
                },
                title,
                content: Some(content),
                metadata: None,
                project_id,
                now,
                topics: input.topics,
                attachment_path: None,
            }
        };
        let created = creator.create(request).await?;
        let summary = async {
            let note = self.db.find_note(&created.inserted.uuid).await?;
            self.summary(note).await
        }
        .await;
        summary.map_err(|error| confirmed_create_followup_error(&created, &error))
    }

    pub async fn create_result(
        &self,
        creator: &dyn NoteCreator,
        input: NoteAddInput,
    ) -> Result<NoteCreateResult, ServiceError> {
        let summary = self.add(creator, input).await?;
        let Some(id) = summary.short_id else {
            return Err(ServiceError::Remote {
                code: "note_create_partial".to_string(),
                message: format!(
                    "Note {} was created, but the backend did not provide a public note ID. Do not create it again.",
                    summary.uuid
                ),
                retryable: false,
                details: Some(serde_json::json!({
                    "created": true,
                    "note_id": summary.uuid,
                    "short_id": null,
                    "confirmed_extraction_ids": [],
                    "pending_extraction_ids": [],
                })),
            });
        };
        Ok(NoteCreateResult { id })
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
        let content = note.content.clone().unwrap_or_default();
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
        self.db.update_note_content(&full_id, &combined).await?;
        self.mutation_result(&full_id, &combined).await
    }

    pub async fn write(
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
        self.db.update_note_content(&full_id, content).await?;
        self.mutation_result(&full_id, content).await
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
        self.db.update_note_content(&full_id, updated).await?;
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
        self.db.update_note_content(&full_id, updated).await?;
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
        self.db.update_note_content(&full_id, updated).await?;
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
        self.db.update_note_content(&full_id, updated).await?;
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
        if !has_edit
            && input.title.is_missing()
            && input.summary.is_missing()
            && input.project.is_missing()
            && input.flagged.is_missing()
        {
            return Err(ServiceError::NothingToModify);
        }

        let full_id = self.db.resolve_note_id(&input.id).await?;
        let note = self.db.find_note(&full_id).await?;
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
                    .update_note_content(&full_id, resulting_content.trim())
                    .await?;
            } else {
                let content = self.required_content(&full_id).await?;
                let matched = edit_match::find_unique(&content, before)?;
                resulting_content = edit_match::splice(&content, &matched, after);
                self.db
                    .update_note_content(&full_id, &resulting_content)
                    .await?;
            }
        }

        self.apply_metadata_patch(&full_id, &note, &input).await?;

        self.mutation_result(&full_id, &resulting_content).await
    }

    pub async fn submit(&self, note_id: &str) -> Result<NoteMutationResult, ServiceError> {
        let full_id = self.db.resolve_note_id(note_id).await?;
        if !self.db.submit_draft(&full_id).await? {
            return Err(ServiceError::NotDraft);
        }
        let content = self
            .db
            .find_note_content(&full_id)
            .await?
            .unwrap_or_default();
        self.mutation_result(&full_id, &content).await
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

    async fn apply_metadata_patch(
        &self,
        note_id: &str,
        note: &crate::types::Note,
        input: &NoteModifyInput,
    ) -> Result<(), ServiceError> {
        match &input.title {
            Patch::Missing => {}
            Patch::Null => self.db.update_note_title(note_id, None).await?,
            Patch::Value(title) => self.db.update_note_title(note_id, Some(title)).await?,
        }
        match &input.summary {
            Patch::Missing => {}
            Patch::Null => self.db.update_note_summary(note_id, None).await?,
            Patch::Value(summary) => self.db.update_note_summary(note_id, Some(summary)).await?,
        }
        self.apply_project_patch(note_id, note, &input.project)
            .await?;
        match &input.flagged {
            Patch::Missing => {}
            Patch::Null => self.db.update_note_flagged(note_id, None).await?,
            Patch::Value(flagged) => self.db.update_note_flagged(note_id, Some(*flagged)).await?,
        }
        Ok(())
    }

    async fn apply_project_patch(
        &self,
        note_id: &str,
        note: &crate::types::Note,
        project: &Patch<String>,
    ) -> Result<(), ServiceError> {
        let Patch::Value(name) = project else {
            if matches!(project, Patch::Null) && note.project_id.is_some() {
                self.db.update_note_project(note_id, None).await?;
            }
            return Ok(());
        };
        let project_id = self
            .db
            .find_project_by_name(name)
            .await?
            .ok_or_else(|| ServiceError::ProjectNotFound(name.to_string()))?;
        if note.project_id.as_deref() != Some(project_id.as_str()) {
            self.db
                .move_note_to_project(note_id, &project_id, note.project_id.as_deref())
                .await?;
        }
        Ok(())
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
        let content_bytes = note
            .content
            .as_ref()
            .map_or(0, |content| content.len() as u64);
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
            title: note.title,
            project_id: note.project_id,
            project,
            topics,
            summary: note.summary,
            content_bytes,
            flagged: note.is_flagged == Some(1),
            draft: note.status == "draft",
            created_at: note.created_at,
            updated_at: note.updated_at,
            deleted_at: note.deleted_at,
        })
    }

    async fn list_item(&self, note: crate::types::Note) -> Result<NoteListItem, ServiceError> {
        let metadata = note
            .metadata
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(crate::error::CliError::Json)?;
        let mut item: NoteListItem = self.summary(note).await?.into();
        item.metadata = metadata;
        Ok(item)
    }
}

pub fn confirmed_create_followup_error(
    created: &crate::services::ports::CreatedNote,
    error: &ServiceError,
) -> ServiceError {
    let inserted = &created.inserted;
    ServiceError::Remote {
        code: "note_create_partial".to_string(),
        message: format!(
            "Note {} was created, but its canonical result could not be loaded: {error}. Do not create it again.",
            inserted
                .short_id
                .map_or_else(|| inserted.uuid.clone(), |id| id.to_string())
        ),
        retryable: false,
        details: Some(serde_json::json!({
            "created": true,
            "note_id": inserted.uuid,
            "short_id": inserted.short_id,
            "confirmed_extraction_ids": created.confirmed_extraction_ids,
            "pending_extraction_ids": [],
        })),
    }
}

#[cfg(all(test, feature = "powersync"))]
mod tests {

    use crate::backend::NoteDb;
    use crate::services::dto::{NoteAddInput, NoteRouteProjectInput, Patch};
    use crate::services::ports::{
        BrowserOpener, CreateNote, CreatedNote, NoteCreator, ShareGateway, ShareResource,
    };
    use crate::services::test_support::{insert_normal_note, make_backend};
    use async_trait::async_trait;

    use super::{
        ExtractionFilterDto, InsertPosition, NoteCountInput, NoteFindInput, NoteListInput,
        NoteModifyInput, NoteService,
    };

    #[tokio::test]
    async fn route_project_rejects_malformed_project_uuid_and_probability() {
        let backend = make_backend().await;
        let service = NoteService::new(&*backend);

        let malformed = service
            .route_project(vec![NoteRouteProjectInput {
                note_id: 1,
                project_id: Some("not-a-uuid".to_string()),
                probability: 0.5,
            }])
            .await
            .unwrap_err();
        assert_eq!(malformed.code(), "invalid_argument");

        let probability = service
            .route_project(vec![NoteRouteProjectInput {
                note_id: 1,
                project_id: None,
                probability: 1.1,
            }])
            .await
            .unwrap_err();
        assert_eq!(probability.code(), "invalid_argument");
    }

    #[tokio::test]
    async fn append_separates_content_and_preserves_lifecycle() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "existing", "synced").await;
        let service = NoteService::new(&*backend);

        let result = service.append(&id, "added").await.unwrap();

        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.content.as_deref(), Some("existing\n\nadded"));
        assert_eq!(note.status, "synced");
        assert_eq!(result.note.uuid, id);
        assert!(result.sections.is_empty());
    }

    #[tokio::test]
    async fn replace_section_replaces_subtree_without_changing_lifecycle() {
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
        let service = NoteService::new(&*backend);

        let result = service
            .replace_section(&id, &section, "# Replacement\nnew")
            .await
            .unwrap();

        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(
            note.content.as_deref(),
            Some("## Replacement\nnew\n\n## Keep\nstable")
        );
        assert_eq!(note.status, "synced");
        assert_eq!(result.sections[0].title, "Replacement");
    }

    #[tokio::test]
    async fn write_replaces_content_without_changing_lifecycle() {
        let backend = make_backend().await;
        let draft_id = insert_normal_note(&backend, "old draft body", "draft").await;
        let synced_id = insert_normal_note(&backend, "old synced body", "synced").await;
        let service = NoteService::new(&*backend);

        let result = service.write(&draft_id, "new draft body").await.unwrap();
        service.write(&synced_id, "new synced body").await.unwrap();

        let note = backend.find_note(&draft_id).await.unwrap();
        assert_eq!(note.content.as_deref(), Some("new draft body"));
        assert_eq!(note.status, "draft");
        assert!(result.note.draft);
        let note = backend.find_note(&synced_id).await.unwrap();
        assert_eq!(note.content.as_deref(), Some("new synced body"));
        assert_eq!(note.status, "synced");
    }

    #[tokio::test]
    async fn write_changes_only_content() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "old body", "draft").await;
        let project_id = backend.create_project("work").await.unwrap();
        backend
            .set_note_extractions(&id, "::topic", &["Preserved topic".to_string()])
            .await
            .unwrap();
        let service = NoteService::new(&*backend);
        service
            .modify(NoteModifyInput {
                id: id.clone(),
                before: None,
                after: None,
                section: None,
                title: Patch::Value("Kept title".to_string()),
                summary: Patch::Value("Kept summary".to_string()),
                project: Patch::Value("work".to_string()),
                flagged: Patch::Value(true),
            })
            .await
            .unwrap();

        service.write(&id, "replacement body").await.unwrap();

        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.content.as_deref(), Some("replacement body"));
        assert_eq!(note.title.as_deref(), Some("Kept title"));
        assert_eq!(note.summary.as_deref(), Some("Kept summary"));
        assert_eq!(note.project_id.as_deref(), Some(project_id.as_str()));
        assert_eq!(note.is_flagged, Some(1));
        assert_eq!(note.status, "draft");
        assert_eq!(
            backend.list_note_topics(&[&id]).await.unwrap()[&id],
            vec!["Preserved topic".to_string()]
        );
    }

    #[tokio::test]
    async fn modify_rejects_ambiguous_before_without_writing() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "same\n\nsame", "synced").await;
        let service = NoteService::new(&*backend);

        let error = service
            .modify(NoteModifyInput {
                id: id.clone(),
                before: Some("same".to_string()),
                after: Some("changed".to_string()),
                section: None,
                title: Patch::Missing,
                summary: Patch::Missing,
                project: Patch::Missing,
                flagged: Patch::Missing,
            })
            .await
            .unwrap_err();

        assert_eq!(error.code(), "before_ambiguous");
        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.content.as_deref(), Some("same\n\nsame"));
        assert_eq!(note.status, "synced");
    }

    #[tokio::test]
    async fn exact_content_modify_preserves_lifecycle() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "old body", "synced").await;
        let service = NoteService::new(&*backend);

        service
            .modify(NoteModifyInput {
                id: id.clone(),
                before: Some("old body".to_string()),
                after: Some("new body".to_string()),
                section: None,
                title: Patch::Missing,
                summary: Patch::Missing,
                project: Patch::Missing,
                flagged: Patch::Missing,
            })
            .await
            .unwrap();

        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.content.as_deref(), Some("new body"));
        assert_eq!(note.status, "synced");
    }

    #[tokio::test]
    async fn exact_content_modify_does_not_expose_editable_document_frontmatter() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "stored body", "ready").await;
        let service = NoteService::new(&*backend);

        let error = service
            .modify(NoteModifyInput {
                id: id.clone(),
                before: Some("title: Test note".to_string()),
                after: Some("title: Changed title".to_string()),
                section: None,
                title: Patch::Missing,
                summary: Patch::Missing,
                project: Patch::Missing,
                flagged: Patch::Missing,
            })
            .await
            .unwrap_err();

        assert_eq!(error.code(), "before_not_found");
        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.title.as_deref(), Some("Test note"));
        assert_eq!(note.content.as_deref(), Some("stored body"));
        assert_eq!(note.status, "ready");
    }

    #[tokio::test]
    async fn archive_and_restore_target_the_explicit_note() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "body", "synced").await;
        let service = NoteService::new(&*backend);

        let archived = service.archive(&id).await.unwrap();
        assert!(archived.archived);
        assert!(backend.find_note(&id).await.is_err());

        let restored = service.restore(&id).await.unwrap();
        assert!(!restored.archived);
        assert_eq!(backend.find_note(&id).await.unwrap().id, id);
    }

    #[tokio::test]
    async fn list_returns_public_item_with_project_name() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "body", "synced").await;
        let project_id = backend.create_project("work").await.unwrap();
        backend
            .move_note_to_project(&id, &project_id, None)
            .await
            .unwrap();
        let service = NoteService::new(&*backend);

        let notes = service
            .list(NoteListInput {
                note_type: None,
                project: Some("work".to_string()),
                no_project: false,
                created_after: None,
                created_before: None,
                archived: false,
                limit: 20,
                cursor: None,
            })
            .await
            .unwrap();

        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, None);
        assert_eq!(notes[0].title.as_deref(), Some("Test note"));
        assert_eq!(notes[0].project.as_deref(), Some("work"));
        assert_eq!(notes[0].content_bytes, 4);
    }

    #[tokio::test]
    async fn content_bytes_counts_stored_unicode_utf8_bytes() {
        let backend = make_backend().await;
        let stored = "---\ncustom: keep\n---\n\n中文😊";
        let id = insert_normal_note(&backend, stored, "synced").await;
        let service = NoteService::new(&*backend);

        let detail = service.get(&id, false).await.unwrap();
        let listed = service
            .list(NoteListInput {
                note_type: None,
                project: None,
                no_project: false,
                created_after: None,
                created_before: None,
                archived: false,
                limit: 20,
                cursor: None,
            })
            .await
            .unwrap();

        assert_eq!(detail.content, stored);
        assert_ne!(stored.len(), stored.chars().count());
        assert_eq!(detail.note.content_bytes, stored.len() as u64);
        assert_eq!(listed[0].content_bytes, stored.len() as u64);
    }

    #[tokio::test]
    async fn get_returns_stored_content_and_section_tree() {
        let backend = make_backend().await;
        let stored = "---\ncustom: preserve\n---\n\n## Part\nBody";
        let id = insert_normal_note(&backend, stored, "synced").await;
        let service = NoteService::new(&*backend);

        let detail = service.get(&id, false).await.unwrap();

        assert_eq!(detail.content, stored);
        assert!(!detail.content.contains("title: Test note"));
        assert!(
            detail
                .sections
                .iter()
                .any(|section| section.title == "Part")
        );
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
        let service = NoteService::new(&*backend);

        let renamed = service
            .rename_section(&id, &section, "Renamed")
            .await
            .unwrap();
        assert_eq!(renamed.sections[0].title, "Renamed");
        assert_eq!(backend.find_note(&id).await.unwrap().status, "synced");
        let renamed_id = renamed.sections[0].id.clone();

        let deleted = service.delete_section(&id, &renamed_id).await.unwrap();
        assert_eq!(deleted.sections.len(), 1);
        assert_eq!(deleted.sections[0].title, "Second");
        assert_eq!(backend.find_note(&id).await.unwrap().status, "synced");
    }

    #[tokio::test]
    async fn insert_after_section_places_content_after_the_whole_subtree() {
        let backend = make_backend().await;
        let original = "## First\none\n\n### Child\nchild\n\n## Second\ntwo";
        let id = insert_normal_note(&backend, original, "synced").await;
        let section = crate::services::markdown::parse_markdown(original).headings[0]
            .id
            .clone();
        let service = NoteService::new(&*backend);

        service
            .insert(&id, &section, InsertPosition::After, "## New\nnew")
            .await
            .unwrap();

        let content = backend.find_note_content(&id).await.unwrap().unwrap();
        assert_eq!(
            content,
            "## First\none\n\n### Child\nchild\n\n## New\nnew\n\n## Second\ntwo"
        );
        assert_eq!(backend.find_note(&id).await.unwrap().status, "synced");
    }

    #[tokio::test]
    async fn find_and_count_use_typed_filters() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "PowerSync notes", "synced").await;
        backend
            .set_note_extractions(&id, "::topic", &["Rust".to_string()])
            .await
            .unwrap();
        let service = NoteService::new(&*backend);

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
        assert_eq!(found[0].id, None);
        assert_eq!(found[0].title.as_deref(), Some("Test note"));
        assert_eq!(found[0].content_bytes, "PowerSync notes".len() as u64);

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
    async fn ranked_ids_keep_order_and_drop_stale_hits() {
        let backend = make_backend().await;
        let first = insert_normal_note(&backend, "first", "synced").await;
        let second = insert_normal_note(&backend, "second", "draft").await;
        backend
            .update_note_title(&first, Some("First"))
            .await
            .unwrap();
        backend
            .update_note_title(&second, Some("Second"))
            .await
            .unwrap();
        let service = NoteService::new(&*backend);

        let found = service
            .find_ranked_ids(&[second.clone(), "stale-id".to_string(), first.clone()])
            .await
            .unwrap();

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title.as_deref(), Some("Second"));
        assert!(found[0].draft);
        assert_eq!(found[1].title.as_deref(), Some("First"));
    }

    #[tokio::test]
    async fn get_section_returns_heading_and_full_subtree() {
        let backend = make_backend().await;
        let original = "## First\none\n\n### Child\nchild\n\n## Second\ntwo";
        let id = insert_normal_note(&backend, original, "synced").await;
        let section = crate::services::markdown::parse_markdown(original).headings[0]
            .id
            .clone();
        let service = NoteService::new(&*backend);

        let result = service.get_section(&id, &section).await.unwrap();

        assert_eq!(result.id, section);
        assert_eq!(result.title, "First");
        assert_eq!(result.content, "## First\none\n\n### Child\nchild");
    }

    #[tokio::test]
    async fn get_section_accepts_an_id_returned_by_get_when_title_matches_heading() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "# Test note\nBody", "synced").await;
        let service = NoteService::new(&*backend);
        let detail = service.get(&id, false).await.unwrap();
        let section_id = detail.sections[0].id.clone();

        let section = service.get_section(&id, &section_id).await.unwrap();

        assert_eq!(section.id, section_id);
        assert_eq!(section.title, "Test note");
        assert_eq!(section.content, "# Test note\nBody");
    }

    #[tokio::test]
    async fn draft_projection_is_consistent_for_list_find_detail_and_mutation() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "draft searchable body", "draft").await;
        let service = NoteService::new(&*backend);

        let listed = service
            .list(NoteListInput {
                note_type: None,
                project: None,
                no_project: false,
                created_after: None,
                created_before: None,
                archived: false,
                limit: 20,
                cursor: None,
            })
            .await
            .unwrap();
        let found = service
            .find(NoteFindInput {
                keywords: vec!["searchable".to_string()],
                extractions: Vec::new(),
                project: None,
                archived: false,
                limit: 20,
            })
            .await
            .unwrap();
        let detail = service.get(&id, false).await.unwrap();
        let mutation = service.append(&id, "more").await.unwrap();

        assert!(listed[0].draft);
        assert!(found[0].draft);
        assert!(detail.note.draft);
        assert!(mutation.note.draft);
    }

    #[tokio::test]
    async fn modify_patches_metadata_without_changing_lifecycle() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "body", "draft").await;
        let project_id = backend.create_project("work").await.unwrap();
        let service = NoteService::new(&*backend);

        let result = service
            .modify(NoteModifyInput {
                id: id.clone(),
                before: None,
                after: None,
                section: None,
                title: Patch::Value("New title".to_string()),
                summary: Patch::Value("Short summary".to_string()),
                project: Patch::Value("work".to_string()),
                flagged: Patch::Value(true),
            })
            .await
            .unwrap();
        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.title.as_deref(), Some("New title"));
        assert_eq!(note.summary.as_deref(), Some("Short summary"));
        assert_eq!(note.project_id.as_deref(), Some(project_id.as_str()));
        assert_eq!(note.is_flagged, Some(1));
        assert_eq!(note.status, "draft");
        assert!(result.note.draft);

        service
            .modify(NoteModifyInput {
                id: id.clone(),
                before: None,
                after: None,
                section: None,
                title: Patch::Null,
                summary: Patch::Null,
                project: Patch::Null,
                flagged: Patch::Null,
            })
            .await
            .unwrap();
        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.title, None);
        assert_eq!(note.summary, None);
        assert_eq!(note.project_id, None);
        assert_eq!(note.is_flagged, None);
        assert_eq!(note.status, "draft");
    }

    #[tokio::test]
    async fn submit_transitions_only_drafts_without_touching_content_or_metadata() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "draft body", "draft").await;
        let service = NoteService::new(&*backend);
        service
            .modify(NoteModifyInput {
                id: id.clone(),
                before: None,
                after: None,
                section: None,
                title: Patch::Value("Kept title".to_string()),
                summary: Patch::Value("Kept summary".to_string()),
                project: Patch::Missing,
                flagged: Patch::Value(true),
            })
            .await
            .unwrap();

        let result = service.submit(&id).await.unwrap();
        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.status, "ai_queued");
        assert_eq!(note.content.as_deref(), Some("draft body"));
        assert_eq!(note.title.as_deref(), Some("Kept title"));
        assert_eq!(note.summary.as_deref(), Some("Kept summary"));
        assert_eq!(note.is_flagged, Some(1));
        assert!(!result.note.draft);

        let error = service.submit(&id).await.unwrap_err();
        assert_eq!(error.code(), "not_draft");
        assert_eq!(error.to_string(), "Note is not a draft");
    }

    #[tokio::test]
    async fn concurrent_submit_succeeds_once() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "draft body", "draft").await;
        let service = NoteService::new(&*backend);

        let (first, second) = tokio::join!(service.submit(&id), service.submit(&id));
        let results = [first, second];

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .map(super::ServiceError::code)
                .collect::<Vec<_>>(),
            ["not_draft"]
        );
        assert_eq!(backend.find_note(&id).await.unwrap().status, "ai_queued");
    }

    #[tokio::test]
    async fn write_rejects_empty_content_without_mutating_the_note() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "kept", "draft").await;
        let service = NoteService::new(&*backend);

        let error = service.write(&id, " \n\t ").await.unwrap_err();

        assert_eq!(error.code(), "invalid_argument");
        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.content.as_deref(), Some("kept"));
        assert_eq!(note.status, "draft");
    }

    #[tokio::test]
    async fn human_edit_save_preserves_lifecycle_status() {
        let backend = make_backend().await;
        for status in ["draft", "ready"] {
            let id = insert_normal_note(&backend, "Original body", status).await;
            let editable = crate::services::editable_document::load_editable_note(&*backend, &id)
                .await
                .unwrap();
            let edited = editable.replace("Original body", "Edited body");

            crate::services::editable_document::save_editable_note(&*backend, &id, &edited)
                .await
                .unwrap();

            let note = backend.find_note(&id).await.unwrap();
            assert_eq!(note.content.as_deref(), Some("Edited body\n"));
            assert_eq!(note.status, status);
        }
    }

    struct DbCreator<'a> {
        db: &'a dyn NoteDb,
        request: std::sync::Mutex<Option<CreateNote>>,
    }

    #[async_trait]
    impl NoteCreator for DbCreator<'_> {
        async fn create(
            &self,
            request: CreateNote,
        ) -> Result<CreatedNote, crate::services::error::ServiceError> {
            let inserted = self.db.insert_note(&request.as_insert_request()).await?;
            *self.request.lock().unwrap() = Some(request);
            Ok(CreatedNote {
                inserted,
                confirmed_extraction_ids: Vec::new(),
            })
        }
    }

    struct DetachedCreator;

    #[async_trait]
    impl NoteCreator for DetachedCreator {
        async fn create(
            &self,
            request: CreateNote,
        ) -> Result<CreatedNote, crate::services::error::ServiceError> {
            Ok(CreatedNote {
                inserted: crate::backend::InsertedNote {
                    uuid: request.id,
                    short_id: Some(42),
                },
                confirmed_extraction_ids: vec!["extraction-confirmed".to_string()],
            })
        }
    }

    #[tokio::test]
    async fn add_reports_structured_partial_when_summary_read_fails_after_create() {
        let backend = make_backend().await;
        let error = NoteService::new(&*backend)
            .add(
                &DetachedCreator,
                NoteAddInput {
                    content: "Body".to_string(),
                    project: None,
                    interpret_as_url: false,
                    draft: false,
                    topics: Vec::new(),
                    created_at: None,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(error.code(), "note_create_partial");
        assert!(!error.retryable());
        let crate::services::error::ServiceError::Remote { details, .. } = error else {
            panic!("expected structured post-create error")
        };
        let details = details.unwrap();
        assert_eq!(details["created"], true);
        assert_eq!(details["short_id"], 42);
        assert!(details["note_id"].as_str().is_some());
        assert_eq!(
            details["confirmed_extraction_ids"],
            serde_json::json!(["extraction-confirmed"])
        );
    }

    #[tokio::test]
    async fn create_result_rejects_a_created_note_without_a_public_id() {
        let backend = make_backend().await;
        let creator = DbCreator {
            db: &*backend,
            request: std::sync::Mutex::new(None),
        };

        let error = NoteService::new(&*backend)
            .create_result(
                &creator,
                NoteAddInput {
                    content: "Body".to_string(),
                    project: None,
                    interpret_as_url: false,
                    draft: false,
                    topics: Vec::new(),
                    created_at: None,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(error.code(), "note_create_partial");
        assert!(!error.retryable());
        let crate::services::error::ServiceError::Remote { details, .. } = error else {
            panic!("expected structured post-create error")
        };
        let details = details.unwrap();
        assert_eq!(details["created"], true);
        assert_eq!(details["short_id"], serde_json::Value::Null);
        assert!(details["note_id"].as_str().is_some());
    }

    #[tokio::test]
    async fn add_normalizes_h1_before_calling_creator() {
        let backend = make_backend().await;
        let creator = DbCreator {
            db: &*backend,
            request: std::sync::Mutex::new(None),
        };
        let service = NoteService::new(&*backend);

        let created = service
            .add(
                &creator,
                NoteAddInput {
                    content: "# Title\n\nBody".to_string(),
                    project: None,
                    interpret_as_url: true,
                    draft: false,
                    topics: Vec::new(),
                    created_at: None,
                },
            )
            .await
            .unwrap();

        let request = creator.request.lock().unwrap();
        let request = request.as_ref().unwrap();
        assert_eq!(request.note_type, "normal");
        assert_eq!(request.status, "ai_queued");
        assert_eq!(request.title.as_deref(), Some("Title"));
        assert_eq!(request.content.as_deref(), Some("Body"));
        assert_eq!(created.title.as_deref(), Some("Title"));
    }

    #[tokio::test]
    async fn add_draft_creates_a_normal_note_in_draft_lifecycle() {
        let backend = make_backend().await;
        let creator = DbCreator {
            db: &*backend,
            request: std::sync::Mutex::new(None),
        };

        let created = NoteService::new(&*backend)
            .add(
                &creator,
                NoteAddInput {
                    content: "https://example.com/draft".to_string(),
                    project: None,
                    interpret_as_url: true,
                    draft: true,
                    topics: Vec::new(),
                    created_at: None,
                },
            )
            .await
            .unwrap();

        let request = creator.request.lock().unwrap();
        let request = request.as_ref().unwrap();
        assert_eq!(request.note_type, "normal");
        assert_eq!(request.status, "draft");
        assert_eq!(
            request.content.as_deref(),
            Some("https://example.com/draft")
        );
        assert!(created.draft);
    }

    #[tokio::test]
    async fn add_draft_rejects_empty_body_after_extracting_title() {
        let backend = make_backend().await;
        let creator = DbCreator {
            db: &*backend,
            request: std::sync::Mutex::new(None),
        };

        let error = NoteService::new(&*backend)
            .add(
                &creator,
                NoteAddInput {
                    content: "# Just a Title\n\n".to_string(),
                    project: None,
                    interpret_as_url: false,
                    draft: true,
                    topics: Vec::new(),
                    created_at: None,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(error.code(), "invalid_argument");
        assert!(creator.request.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn add_only_treats_a_pure_http_value_as_a_link() {
        let backend = make_backend().await;
        let creator = DbCreator {
            db: &*backend,
            request: std::sync::Mutex::new(None),
        };

        NoteService::new(&*backend)
            .add(
                &creator,
                NoteAddInput {
                    content: "https://example.com with context".to_string(),
                    project: None,
                    interpret_as_url: true,
                    draft: false,
                    topics: Vec::new(),
                    created_at: None,
                },
            )
            .await
            .unwrap();

        let request = creator.request.lock().unwrap();
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
        let writer = backend.database().writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET source = ? WHERE id = ?",
                rusqlite::params![r#"{"link":{"content":"one\ntwo"}}"#, id],
            )
            .unwrap();
        drop(writer);
        let service = NoteService::new(&*backend);
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
        shared: std::sync::Mutex<Vec<(ShareResource, String)>>,
        opened: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ShareGateway for FakeSideEffects {
        async fn share(
            &self,
            resource: ShareResource,
            id: &str,
        ) -> Result<String, crate::services::error::ServiceError> {
            self.shared.lock().unwrap().push((resource, id.to_string()));
            Ok(format!("https://share.example/{id}"))
        }

        async fn unshare(
            &self,
            resource: ShareResource,
            id: &str,
        ) -> Result<(), crate::services::error::ServiceError> {
            self.shared.lock().unwrap().push((resource, id.to_string()));
            Ok(())
        }
    }

    impl BrowserOpener for FakeSideEffects {
        fn open(&self, url: &str) -> Result<(), crate::services::error::ServiceError> {
            self.opened.lock().unwrap().push(url.to_string());
            Ok(())
        }
    }

    #[tokio::test]
    async fn share_and_open_resolve_note_identity_before_side_effects() {
        let backend = make_backend().await;
        let id = insert_normal_note(&backend, "body", "synced").await;
        let writer = backend.database().writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET short_id = ? WHERE id = ?",
                rusqlite::params![42, id],
            )
            .unwrap();
        drop(writer);
        let side_effects = FakeSideEffects::default();
        let service = NoteService::new(&*backend);

        let shared = service.share(&side_effects, &id).await.unwrap();
        assert_eq!(shared.url, format!("https://share.example/{id}"));
        assert_eq!(
            side_effects.shared.lock().unwrap().as_slice(),
            &[(ShareResource::Note, id.clone())]
        );

        let unshared = service.unshare(&side_effects, &id).await.unwrap();
        assert!(unshared.revoked);

        let opened = service
            .open(&side_effects, "https://app.example/", "42")
            .await
            .unwrap();
        assert_eq!(opened.url, "https://app.example/notes/42");
        assert!(!opened.url.contains(&id));
        assert!(opened.opened);
        assert_eq!(
            side_effects.opened.lock().unwrap().as_slice(),
            &[opened.url]
        );
    }
}
