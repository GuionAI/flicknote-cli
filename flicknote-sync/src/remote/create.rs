use crate::*;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreateNoteRequest {
    pub(crate) id: String,
    pub(crate) note_type: String,
    pub(crate) status: String,
    pub(crate) title: Option<String>,
    pub(crate) content: Option<String>,
    pub(crate) metadata: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) now: String,
    pub(crate) topics: Vec<String>,
    pub(crate) attachment_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteCreatedNote {
    pub(crate) uuid: String,
    pub(crate) short_id: i64,
    pub(crate) confirmed_extraction_ids: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ExtractionCreateOutcome {
    pub(crate) confirmed_ids: Vec<String>,
    pub(crate) pending_ids: Vec<String>,
    pub(crate) diagnostic: Option<String>,
    pub(crate) local_commit_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RemoteNoteRow {
    pub(crate) id: String,
    pub(crate) short_id: Option<i64>,
    pub(crate) user_id: String,
    #[serde(rename = "type")]
    pub(crate) note_type: String,
    pub(crate) status: String,
    pub(crate) title: Option<String>,
    pub(crate) content: Option<String>,
    pub(crate) summary: Option<String>,
    #[serde(default)]
    pub(crate) is_flagged: bool,
    pub(crate) project_id: Option<String>,
    pub(crate) metadata: Option<serde_json::Value>,
    pub(crate) source: Option<serde_json::Value>,
    pub(crate) created_at: Option<String>,
    pub(crate) updated_at: Option<String>,
    pub(crate) deleted_at: Option<String>,
}

pub(crate) fn json_column(
    value: &Option<serde_json::Value>,
) -> Result<Option<String>, DaemonError> {
    value
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to serialize canonical remote JSON: {error}"),
        })
}

pub(crate) async fn commit_remote_note(
    db: &PowerSyncDatabase,
    note: &RemoteNoteRow,
) -> Result<bool, DaemonError> {
    let metadata = json_column(&note.metadata)?;
    let source = json_column(&note.source)?;
    let mut writer = db.writer().await.map_err(|error| DaemonError::Other {
        message: format!("Failed to open local PowerSync writer: {error}"),
    })?;
    let tx = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to begin local note transaction: {error}"),
        })?;
    let exists = tx
        .query_row(
            "SELECT 1 FROM notes WHERE id = ? LIMIT 1",
            params![note.id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to check local note {}: {error}", note.id),
        })?
        .is_some();
    if exists {
        tx.commit().map_err(|error| DaemonError::Other {
            message: format!("Failed to finish local note transaction: {error}"),
        })?;
        return Ok(false);
    }

    tx.execute(
        r#"INSERT INTO notes (
            id, short_id, user_id, type, status, title, content, summary,
            is_flagged, project_id, metadata, source, created_at, updated_at,
            deleted_at, _metadata
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        params![
            note.id,
            note.short_id,
            note.user_id,
            note.note_type,
            note.status,
            note.title,
            note.content,
            note.summary,
            note.is_flagged,
            note.project_id,
            metadata,
            source,
            note.created_at,
            note.updated_at,
            note.deleted_at,
            REMOTE_COMMITTED_INSERT_METADATA,
        ],
    )
    .map_err(|error| DaemonError::Other {
        message: format!("Failed to commit remote note {} locally: {error}", note.id),
    })?;
    tx.commit().map_err(|error| DaemonError::Other {
        message: format!("Failed to finish local note transaction: {error}"),
    })?;
    Ok(true)
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub(crate) struct RemoteExtractionRow {
    pub(crate) id: String,
    pub(crate) note_id: String,
    pub(crate) user_id: String,
    pub(crate) key: String,
    pub(crate) value: String,
}

pub(crate) async fn commit_remote_extractions(
    db: &PowerSyncDatabase,
    rows: &[RemoteExtractionRow],
) -> Result<usize, DaemonError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let mut writer = db.writer().await.map_err(|error| DaemonError::Other {
        message: format!("Failed to open local PowerSync writer: {error}"),
    })?;
    let tx = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to begin local extraction transaction: {error}"),
        })?;
    let mut inserted = 0;
    for row in rows {
        let exists = tx
            .query_row(
                "SELECT 1 FROM note_extractions WHERE id = ? LIMIT 1",
                params![row.id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| DaemonError::Other {
                message: format!("Failed to check local extraction {}: {error}", row.id),
            })?
            .is_some();
        if exists {
            continue;
        }
        tx.execute(
            r#"INSERT INTO note_extractions (
                id, note_id, user_id, key, value, _metadata
            ) VALUES (?, ?, ?, ?, ?, ?)"#,
            params![
                row.id,
                row.note_id,
                row.user_id,
                row.key,
                row.value,
                REMOTE_COMMITTED_INSERT_METADATA,
            ],
        )
        .map_err(|error| DaemonError::Other {
            message: format!(
                "Failed to commit remote extraction {} locally: {error}",
                row.id
            ),
        })?;
        inserted += 1;
    }
    tx.commit().map_err(|error| DaemonError::Other {
        message: format!("Failed to finish local extraction transaction: {error}"),
    })?;
    Ok(inserted)
}

pub(crate) async fn create_note_remotely(
    db: &PowerSyncDatabase,
    http: &reqwest::Client,
    auth: &GoTrueClient,
    config: &Config,
    req: CreateNoteRequest,
) -> Result<RemoteCreatedNote, DaemonError> {
    let session = auth.get_session().await.map_err(|e| DaemonError::Other {
        message: format!("Auth error: {e}"),
    })?;

    create_note_with_token(
        db,
        http,
        config,
        &session.access_token,
        &session.user.id,
        req,
    )
    .await
}

pub(crate) async fn create_note_with_token(
    db: &PowerSyncDatabase,
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    user_id: &str,
    req: CreateNoteRequest,
) -> Result<RemoteCreatedNote, DaemonError> {
    let extraction_rows = req
        .topics
        .iter()
        .map(|value| RemoteExtractionRow {
            id: uuid::Uuid::new_v4().to_string(),
            note_id: req.id.clone(),
            user_id: user_id.to_string(),
            key: TOPIC_EXTRACTION_KEY.to_string(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    let metadata = match req.metadata.as_deref() {
        Some(raw) => {
            serde_json::from_str::<serde_json::Value>(raw).map_err(|e| DaemonError::Other {
                message: format!("Invalid note metadata JSON: {e}"),
            })?
        }
        None => serde_json::Value::Null,
    };

    let attachment_path = req.attachment_path.as_deref().map(Path::new);
    if let Some(path) = attachment_path {
        upload_attachment(http, config, access_token, &req.id, path).await?;
    }

    let payload = serde_json::json!({
        "id": req.id,
        "user_id": user_id,
        "type": req.note_type,
        "status": req.status,
        "title": req.title,
        "content": req.content,
        "metadata": metadata,
        "project_id": req.project_id,
        "created_at": req.now,
        "updated_at": req.now,
    });

    let send_create = || {
        http.post(format!(
            "{}/rest/v1/notes?on_conflict=id",
            config.supabase_url
        ))
        .header("apikey", &config.supabase_anon_key)
        .bearer_auth(access_token)
        .header(
            "Prefer",
            "resolution=ignore-duplicates,return=representation",
        )
        .json(&payload)
        .send()
    };
    let (resp, initial_ambiguous_error) = match send_create().await {
        Ok(resp) if !is_ambiguous_create_status(resp.status()) => (resp, None),
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let initial_error = format!("the first attempt returned {status}: {body}");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            match send_create().await {
                Ok(resp) if !is_ambiguous_create_status(resp.status()) => {
                    (resp, Some(initial_error))
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    if let Ok(Some(row)) =
                        lookup_remote_note(http, config, access_token, &req.id).await
                    {
                        return finish_remote_create(
                            db,
                            http,
                            config,
                            access_token,
                            row,
                            &extraction_rows,
                        )
                        .await;
                    }
                    return Err(ambiguous_create_error(
                        format!(
                            "Remote note create outcome is unknown for note {} after retrying the same stable UUID ({initial_error}; retry returned {status}: {body}). The attachment was retained. Do not create it again.",
                            req.id
                        ),
                        req.id,
                        &extraction_rows,
                    ));
                }
                Err(retry_error) => {
                    if let Ok(Some(row)) =
                        lookup_remote_note(http, config, access_token, &req.id).await
                    {
                        return finish_remote_create(
                            db,
                            http,
                            config,
                            access_token,
                            row,
                            &extraction_rows,
                        )
                        .await;
                    }
                    return Err(ambiguous_create_error(
                        format!(
                            "Remote note create outcome is unknown for note {} after retrying the same stable UUID ({initial_error}; retry: {retry_error}). The attachment was retained. Do not create it again.",
                            req.id
                        ),
                        req.id,
                        &extraction_rows,
                    ));
                }
            }
        }
        Err(initial_error) => {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            match send_create().await {
                Ok(resp) => (resp, Some(initial_error.to_string())),
                Err(retry_error) => {
                    if let Ok(Some(row)) =
                        lookup_remote_note(http, config, access_token, &req.id).await
                    {
                        return finish_remote_create(
                            db,
                            http,
                            config,
                            access_token,
                            row,
                            &extraction_rows,
                        )
                        .await;
                    }
                    return Err(ambiguous_create_error(
                        format!(
                            "Remote note create outcome is unknown for note {} after retrying the same stable UUID ({initial_error}; retry: {retry_error}). The attachment was retained. Do not create it again.",
                            req.id
                        ),
                        req.id,
                        &extraction_rows,
                    ));
                }
            }
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if let Ok(Some(row)) = lookup_remote_note(http, config, access_token, &req.id).await {
            return finish_remote_create(db, http, config, access_token, row, &extraction_rows)
                .await;
        }
        if let Some(initial_error) = initial_ambiguous_error {
            return Err(ambiguous_create_error(
                format!(
                    "Remote note create outcome is unknown for note {} after retrying the same stable UUID: {initial_error}; the retry returned {status}: {body}. The attachment was retained. Do not create it again.",
                    req.id
                ),
                req.id,
                &extraction_rows,
            ));
        }
        if attachment_path.is_some()
            && let Err(e) = delete_attachment(http, config, access_token, &req.id).await
        {
            log::warn!("Failed to clean up uploaded attachment after note create failure: {e}");
        }
        return Err(DaemonError::Other {
            message: format!("Remote note create failed ({status}): {body}"),
        });
    }

    let row = match resp.json::<Vec<RemoteNoteRow>>().await {
        Ok(mut rows) => match rows.pop() {
            Some(row) => row,
            None => {
                reconcile_confirmed_remote_note(
                    http,
                    config,
                    access_token,
                    &req.id,
                    &extraction_rows,
                    format!("Remote note create returned no row for note {}", req.id),
                )
                .await?
            }
        },
        Err(error) => {
            reconcile_confirmed_remote_note(
                http,
                config,
                access_token,
                &req.id,
                &extraction_rows,
                format!("Failed to parse remote note create response: {error}"),
            )
            .await?
        }
    };
    finish_remote_create(db, http, config, access_token, row, &extraction_rows).await
}

pub(crate) fn is_ambiguous_create_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

pub(crate) fn confirmed_create_error(
    message: String,
    note_id: String,
    short_id: Option<i64>,
    extraction_rows: &[RemoteExtractionRow],
) -> DaemonError {
    DaemonError::PartialCreate {
        message,
        note_id,
        short_id,
        confirmed_extraction_ids: Vec::new(),
        pending_extraction_ids: extraction_rows.iter().map(|row| row.id.clone()).collect(),
    }
}

pub(crate) fn partial_create_error(
    message: String,
    note_id: String,
    short_id: Option<i64>,
    confirmed_extraction_ids: Vec<String>,
    pending_extraction_ids: Vec<String>,
) -> DaemonError {
    DaemonError::PartialCreate {
        message,
        note_id,
        short_id,
        confirmed_extraction_ids,
        pending_extraction_ids,
    }
}

pub(crate) fn ambiguous_create_error(
    message: String,
    note_id: String,
    extraction_rows: &[RemoteExtractionRow],
) -> DaemonError {
    DaemonError::AmbiguousCreate {
        message,
        note_id,
        pending_extraction_ids: extraction_rows.iter().map(|row| row.id.clone()).collect(),
    }
}

pub(crate) async fn reconcile_confirmed_remote_note(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    note_id: &str,
    extraction_rows: &[RemoteExtractionRow],
    original_error: String,
) -> Result<RemoteNoteRow, DaemonError> {
    match lookup_remote_note(http, config, access_token, note_id).await {
        Ok(Some(row)) => Ok(row),
        Ok(None) => Err(confirmed_create_error(
            format!(
                "Note {note_id} was accepted remotely, but its canonical row could not be recovered: {original_error}. Do not create it again."
            ),
            note_id.to_string(),
            None,
            extraction_rows,
        )),
        Err(error) => Err(confirmed_create_error(
            format!(
                "Note {note_id} was accepted remotely, but its canonical row could not be recovered: {original_error}; reconciliation failed: {error}. Do not create it again."
            ),
            note_id.to_string(),
            None,
            extraction_rows,
        )),
    }
}

pub(crate) async fn finish_remote_create(
    db: &PowerSyncDatabase,
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    row: RemoteNoteRow,
    extraction_rows: &[RemoteExtractionRow],
) -> Result<RemoteCreatedNote, DaemonError> {
    let short_id = match row.short_id {
        Some(short_id) => short_id,
        None => {
            return Err(confirmed_create_error(
                format!(
                    "Note {} was created remotely, but the backend returned no short id. Do not create it again.",
                    row.id
                ),
                row.id,
                None,
                extraction_rows,
            ));
        }
    };
    if let Err(error) = commit_remote_note(db, &row).await {
        return Err(confirmed_create_error(
            format!(
                "Note {short_id} was created remotely, but could not be committed locally: {error}. Do not create it again."
            ),
            row.id,
            Some(short_id),
            extraction_rows,
        ));
    }
    let extraction_outcome =
        create_extractions_with_token(db, http, config, access_token, extraction_rows).await;
    if !extraction_outcome.pending_ids.is_empty() || extraction_outcome.local_commit_error.is_some()
    {
        let reason = extraction_outcome
            .local_commit_error
            .as_deref()
            .or(extraction_outcome.diagnostic.as_deref())
            .unwrap_or("one or more extraction rows could not be confirmed");
        return Err(partial_create_error(
            format!(
                "Note {short_id} was created, but its topics were not fully committed: {reason}"
            ),
            row.id,
            Some(short_id),
            extraction_outcome.confirmed_ids,
            extraction_outcome.pending_ids,
        ));
    }
    Ok(RemoteCreatedNote {
        uuid: row.id,
        short_id,
        confirmed_extraction_ids: extraction_outcome.confirmed_ids,
    })
}

pub(crate) async fn lookup_remote_note(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    id: &str,
) -> Result<Option<RemoteNoteRow>, DaemonError> {
    let response = http
        .get(format!(
            "{}/rest/v1/notes?id=eq.{id}&select=*",
            config.supabase_url
        ))
        .header("apikey", &config.supabase_anon_key)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to reconcile remote note {id}: {error}"),
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(DaemonError::Other {
            message: format!("Failed to reconcile remote note {id} ({status}): {body}"),
        });
    }
    let mut rows = response
        .json::<Vec<RemoteNoteRow>>()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to parse remote note reconciliation response: {error}"),
        })?;
    Ok(rows.pop())
}

pub(crate) async fn create_extractions_with_token(
    db: &PowerSyncDatabase,
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    requested: &[RemoteExtractionRow],
) -> ExtractionCreateOutcome {
    if requested.is_empty() {
        return ExtractionCreateOutcome::default();
    }

    let response = http
        .post(format!(
            "{}/rest/v1/note_extractions?on_conflict=id",
            config.supabase_url
        ))
        .header("apikey", &config.supabase_anon_key)
        .bearer_auth(access_token)
        .header(
            "Prefer",
            "resolution=ignore-duplicates,return=representation",
        )
        .json(requested)
        .send()
        .await;
    let (mut rows, mut diagnostics) = match response {
        Ok(response) if response.status().is_success() => {
            match response.json::<Vec<RemoteExtractionRow>>().await {
                Ok(rows) => (rows, Vec::new()),
                Err(error) => (
                    Vec::new(),
                    vec![format!(
                        "failed to parse remote extraction create response: {error}"
                    )],
                ),
            }
        }
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            (
                Vec::new(),
                vec![format!(
                    "remote extraction create returned {status}: {body}"
                )],
            )
        }
        Err(error) => (
            Vec::new(),
            vec![format!(
                "remote extraction create failed in transport: {error}"
            )],
        ),
    };
    let mut confirmed_ids = rows
        .iter()
        .map(|row| row.id.clone())
        .collect::<std::collections::HashSet<_>>();
    for expected in requested {
        if confirmed_ids.contains(&expected.id) {
            continue;
        }
        match lookup_remote_extraction(http, config, access_token, &expected.id).await {
            Ok(Some(row)) => {
                rows.push(row);
                confirmed_ids.insert(expected.id.clone());
            }
            Ok(None) => {}
            Err(error) => diagnostics.push(error.to_string()),
        }
    }
    let confirmed_ids = requested
        .iter()
        .filter(|row| confirmed_ids.contains(&row.id))
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();
    let pending_ids = requested
        .iter()
        .filter(|row| !confirmed_ids.contains(&row.id))
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();
    let local_commit_error = if rows.is_empty() {
        None
    } else {
        commit_remote_extractions(db, &rows)
            .await
            .err()
            .map(|error| error.to_string())
    };
    ExtractionCreateOutcome {
        confirmed_ids,
        pending_ids,
        diagnostic: (!diagnostics.is_empty()).then(|| diagnostics.join("; ")),
        local_commit_error,
    }
}

pub(crate) async fn lookup_remote_extraction(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    id: &str,
) -> Result<Option<RemoteExtractionRow>, DaemonError> {
    let response = http
        .get(format!(
            "{}/rest/v1/note_extractions?id=eq.{id}&select=*",
            config.supabase_url
        ))
        .header("apikey", &config.supabase_anon_key)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to reconcile remote extraction {id}: {error}"),
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(DaemonError::Other {
            message: format!("Failed to reconcile remote extraction {id} ({status}): {body}"),
        });
    }
    let mut rows = response
        .json::<Vec<RemoteExtractionRow>>()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to parse extraction reconciliation response: {error}"),
        })?;
    Ok(rows.pop())
}

pub(crate) struct RemoteNoteCreator {
    pub(crate) db: PowerSyncDatabase,
    pub(crate) auth: Arc<GoTrueClient>,
    pub(crate) http: reqwest::Client,
    pub(crate) config: Arc<Config>,
}

pub(crate) fn remote_create_service_error(
    error: DaemonError,
) -> flicknote_core::services::error::ServiceError {
    match error {
        DaemonError::PartialCreate {
            message,
            note_id,
            short_id,
            confirmed_extraction_ids,
            pending_extraction_ids,
        } => flicknote_core::services::error::ServiceError::Remote {
            code: "note_create_partial".to_string(),
            message,
            retryable: false,
            details: Some(serde_json::json!({
                "created": true,
                "note_id": note_id,
                "short_id": short_id,
                "confirmed_extraction_ids": confirmed_extraction_ids,
                "pending_extraction_ids": pending_extraction_ids,
            })),
        },
        DaemonError::AmbiguousCreate {
            message,
            note_id,
            pending_extraction_ids,
        } => flicknote_core::services::error::ServiceError::Remote {
            code: "note_create_unknown".to_string(),
            message,
            retryable: false,
            details: Some(serde_json::json!({
                "created": serde_json::Value::Null,
                "note_id": note_id,
                "short_id": serde_json::Value::Null,
                "pending_extraction_ids": pending_extraction_ids,
            })),
        },
        error => flicknote_core::services::error::ServiceError::Daemon(error.to_string()),
    }
}

#[async_trait]
impl NoteCreator for RemoteNoteCreator {
    async fn create(
        &self,
        request: CreateNote,
    ) -> Result<
        flicknote_core::services::ports::CreatedNote,
        flicknote_core::services::error::ServiceError,
    > {
        let created = create_note_remotely(
            &self.db,
            &self.http,
            &self.auth,
            &self.config,
            CreateNoteRequest {
                id: request.id,
                note_type: request.note_type,
                status: request.status,
                title: request.title,
                content: request.content,
                metadata: request.metadata,
                project_id: request.project_id,
                now: request.now,
                topics: request.topics,
                attachment_path: request.attachment_path,
            },
        )
        .await
        .map_err(remote_create_service_error)?;
        Ok(flicknote_core::services::ports::CreatedNote {
            inserted: flicknote_core::backend::InsertedNote {
                uuid: created.uuid,
                short_id: Some(created.short_id),
            },
            confirmed_extraction_ids: created.confirmed_extraction_ids,
        })
    }
}
