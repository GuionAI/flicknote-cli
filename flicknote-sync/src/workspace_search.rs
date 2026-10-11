//! GUI-local search projection. Public search contracts and ranking stay with Application.
use crate::{
    app::Application,
    today::{TodayRow, fold_preview, persisted_preview, source_url},
};
use flicknote_client::{
    AppRequest, AppResponse, WireError,
    dto::{NoteDetail, NoteFindInput, SearchHit},
};
use powersync::PowerSyncDatabase;
use std::sync::Arc;

pub const LIMIT: u32 = 50;
#[derive(Clone, Debug)]
pub struct Results {
    pub rows: Arc<Vec<TodayRow>>,
    pub hits: Vec<SearchHit>,
    pub detail: Option<i64>,
    pub bounded: bool,
}

pub fn exact_id(query: &str) -> Option<i64> {
    let text = query.trim().strip_prefix('#').unwrap_or(query.trim());
    (!text.is_empty() && text.bytes().all(|c| c.is_ascii_digit()))
        .then(|| text.parse::<i64>().ok().filter(|id| *id > 0))
        .flatten()
}

async fn exact(app: &Application, id: i64) -> Result<Option<NoteDetail>, WireError> {
    for archived in [false, true] {
        match app
            .handle(AppRequest::NoteGet {
                id: id.to_string(),
                archived,
            })
            .await
        {
            Ok(AppResponse::NoteDetail(note)) => return Ok(Some(note)),
            Err(error) if error.code == "note_not_found" => {}
            Err(error) => return Err(error),
            Ok(_) => unreachable!("NoteGet contract"),
        }
    }
    Ok(None)
}

/// Resolve at most 50 identities/previews, then read only the explicitly selected body.
/// The actual app enforces current-account and lifecycle access for every full read.
pub async fn read(
    app: &Application,
    db: &PowerSyncDatabase,
    user: &str,
    query: &str,
    human: bool,
    selected: Option<i64>,
) -> Result<Results, String> {
    let exact = match exact_id(query) {
        Some(id) => exact(app, id).await.map_err(|e| e.message)?,
        None => None,
    };
    let AppResponse::SearchHits(mut hits) = app
        .handle(AppRequest::NoteFind(NoteFindInput {
            keywords: vec![query.to_owned()],
            extractions: vec![],
            project: None,
            created_after: None,
            created_before: None,
            human,
            archived: false,
            limit: LIMIT,
        }))
        .await
        .map_err(|e| e.message)?
    else {
        unreachable!("NoteFind contract")
    };
    if let Some(note) = &exact {
        let lexical = hits
            .iter()
            .position(|hit| hit.short_id == note.note.short_id)
            .map(|index| hits.remove(index));
        hits.insert(
            0,
            lexical.unwrap_or_else(|| SearchHit {
                short_id: note.note.short_id,
                note_type: note.note.note_type.clone(),
                content_bytes: note.note.content_bytes,
                draft: note.note.draft,
                title: note.note.title.clone(),
                summary: note.note.summary.clone(),
                created_at: note.note.created_at.clone(),
                updated_at: note.note.updated_at.clone(),
                project_id: note.note.project_id.clone(),
                snippet: Default::default(),
            }),
        );
    }
    let bounded = hits.len() >= LIMIT as usize;
    hits.truncate(LIMIT as usize);
    let mut rows = project(
        db,
        user,
        &hits,
        exact.as_ref().map(|n| n.note.uuid.as_str()),
    )
    .await?;
    let mut detail = None;
    if let Some(row) = rows.iter_mut().find(|r| Some(r.id) == selected) {
        match app
            .handle(AppRequest::NoteGet {
                id: row.uuid.clone(),
                archived: row.archived,
            })
            .await
        {
            Ok(AppResponse::NoteDetail(note))
                if note.note.uuid == row.uuid && note.note.short_id == Some(row.id) =>
            {
                row.source_url = source_url(
                    &note.note.note_type,
                    note.metadata
                        .as_ref()
                        .map(serde_json::Value::to_string)
                        .as_deref(),
                );
                row.note_type = note.note.note_type;
                row.content = note.content;
                row.title = note.note.title;
                row.project_id = note.note.project_id;
                row.project_name = note.note.project;
                row.draft = note.note.draft;
                row.archived = note.note.deleted_at.is_some();
                detail = Some(row.id);
            }
            Err(error) if error.code == "note_not_found" => {
                rows.retain(|r| Some(r.id) != selected);
            }
            Err(error) => return Err(error.message),
            Ok(_) => return Err("Note identity changed; search again".into()),
        }
    }
    Ok(Results {
        rows: Arc::new(rows),
        hits,
        detail,
        bounded,
    })
}

async fn project(
    db: &PowerSyncDatabase,
    user: &str,
    hits: &[SearchHit],
    exact_uuid: Option<&str>,
) -> Result<Vec<TodayRow>, String> {
    let reader = db.reader().await.map_err(|e| e.to_string())?;
    let mut stmt = reader.prepare_cached(
        "SELECT n.id, CASE WHEN length(CAST(coalesce(n.content, '') AS BLOB)) <= 512 THEN coalesce(n.content, '') ELSE coalesce(n.title, 'Untitled note') END, n.title, n.project_id, p.name, p.color, coalesce(n.type, 'normal'), n.deleted_at IS NOT NULL, coalesce(n.status, '') = 'draft', EXISTS (SELECT 1 FROM note_shares s WHERE s.id = n.id AND s.user_id = n.user_id AND (s.expires_at IS NULL OR julianday(s.expires_at) > julianday('now'))) , n.metadata, coalesce(n.content, '') = '' FROM notes n LEFT JOIN projects p ON p.id = n.project_id AND p.user_id = n.user_id WHERE n.user_id = ?1 AND n.short_id = ?2"
    ).map_err(|e| e.to_string())?;
    let mut rows = Vec::with_capacity(hits.len());
    for hit in hits {
        let Some(id) = hit.short_id else {
            continue;
        };
        let mut found = stmt
            .query(rusqlite::params![user, id])
            .map_err(|e| e.to_string())?;
        let Some(r) = found.next().map_err(|e| e.to_string())? else {
            continue;
        };
        let row = (|| -> rusqlite::Result<TodayRow> {
            let note_type: String = r.get(6)?;
            let source_url = source_url(&note_type, r.get::<_, Option<String>>(10)?.as_deref());
            let title: Option<String> = r.get(2)?;
            let preview = if r.get::<_, bool>(11)? {
                persisted_preview("", title.as_deref(), source_url.as_deref(), &note_type)
            } else {
                fold_preview(&r.get::<_, String>(1)?)
            };
            Ok(TodayRow {
                id,
                uuid: r.get(0)?,
                preview,
                title,
                project_id: r.get(3)?,
                project_name: r.get(4)?,
                project_color: r.get(5)?,
                note_type,
                source_url,
                archived: r.get(7)?,
                draft: r.get(8)?,
                shared: r.get(9)?,
                failed_stage: None,
                content: String::new(),
            })
        })()
        .map_err(|e| e.to_string())?;
        let is_exact = exact_uuid == Some(row.uuid.as_str());
        if (row.archived || row.draft) && !is_exact {
            continue;
        }
        rows.push(row);
    }
    Ok(rows)
}
