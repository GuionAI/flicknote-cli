//! Process-owned, bounded feedback for the workspace's five explicit note actions.
use super::*;
use flicknote_sync::today::{ProjectContext, TodayRow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NoteAction {
    Project(ProjectContextIdentity),
    Archive,
    Share,
    Unshare,
    Restore,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProjectContextIdentity {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
}
impl From<&ProjectContext> for ProjectContextIdentity {
    fn from(project: &ProjectContext) -> Self {
        Self {
            id: project.id.clone(),
            name: project.name.clone(),
            color: project.color.clone(),
        }
    }
}
#[derive(Default)]
pub(crate) struct NoteActions {
    pending: Vec<Accepted>,
    next_token: u64,
    pub(crate) error: Option<String>,
    pub(crate) links: Vec<String>,
}
struct Accepted {
    token: u64,
    row: TodayRow,
    action: NoteAction,
    scope: (Destination, bool),
    projecting: bool,
    complete: bool,
}
impl NoteActions {
    pub(super) fn busy(&self, uuid: &str) -> bool {
        self.pending.iter().any(|p| p.row.uuid == uuid)
    }
    fn accept(
        &mut self,
        row: TodayRow,
        action: NoteAction,
        scope: (Destination, bool),
    ) -> Option<u64> {
        if self.busy(&row.uuid) {
            return None;
        }
        self.next_token += 1;
        self.error = None;
        self.pending.push(Accepted {
            token: self.next_token,
            row,
            action,
            scope,
            projecting: true,
            complete: false,
        });
        Some(self.next_token)
    }
    pub(super) fn observe(&mut self, rows: &[TodayRow], scope: &(Destination, bool)) {
        for pending in &mut self.pending {
            let row = rows.iter().find(|r| r.uuid == pending.row.uuid);
            let changed = row.is_some_and(|row| match &pending.action {
                NoteAction::Project(_) => row.project_id != pending.row.project_id,
                NoteAction::Archive | NoteAction::Restore => row.archived != pending.row.archived,
                NoteAction::Share | NoteAction::Unshare => row.shared != pending.row.shared,
            });
            if changed || (row.is_none() && scope == &pending.scope) {
                // Includes unrelated external writers. Never resurrect an older overlay.
                pending.projecting = false;
            }
        }
        self.pending.retain(|p| !p.complete || p.projecting);
    }
    pub(super) fn project(
        &self,
        rows: &[TodayRow],
        destination: &Destination,
    ) -> Arc<Vec<TodayRow>> {
        Arc::new(
            rows.iter()
                .filter_map(|row| {
                    let Some(pending) = self
                        .pending
                        .iter()
                        .find(|p| p.row.uuid == row.uuid && p.projecting)
                    else {
                        return Some(row.clone());
                    };
                    match &pending.action {
                        NoteAction::Archive | NoteAction::Restore => None,
                        NoteAction::Unshare if destination == &Destination::Shared => None,
                        NoteAction::Share if pending.complete => {
                            let mut row = row.clone();
                            row.shared = true;
                            Some(row)
                        }
                        NoteAction::Project(project) => {
                            if let Destination::Project(id) = destination
                                && id != &project.id
                            {
                                return None;
                            }
                            let mut row = row.clone();
                            row.project_id = Some(project.id.clone());
                            row.project_name = Some(project.name.clone());
                            row.project_color.clone_from(&project.color);
                            Some(row)
                        }
                        _ => Some(row.clone()),
                    }
                })
                .collect(),
        )
    }
    fn complete(
        &mut self,
        token: u64,
        result: Result<Option<String>, flicknote_client::WireError>,
    ) {
        let Some(index) = self.pending.iter().position(|p| p.token == token) else {
            return;
        };
        match result {
            Ok(link) => {
                self.links.extend(link);
                if matches!(self.pending[index].action, NoteAction::Share)
                    && self.pending[index].row.shared
                {
                    // Already watched shared: Copy Link needs no confirmation overlay.
                    self.pending[index].projecting = false;
                }
                self.pending[index].complete = true;
                if !self.pending[index].projecting {
                    self.pending.remove(index);
                }
            }
            Err(error) => {
                let verb = match self.pending.remove(index).action {
                    NoteAction::Project(_) => "classify",
                    NoteAction::Archive => "archive",
                    NoteAction::Share => "share",
                    NoteAction::Unshare => "unshare",
                    NoteAction::Restore => "restore",
                };
                let definite = matches!(
                    error.code.as_str(),
                    "invalid_argument" | "project_not_found" | "note_not_found"
                );
                self.error = Some(if definite {
                    format!("Could not {verb}: {}", error.message)
                } else {
                    format!(
                        "{} Check the note before trying again; the change may have completed.",
                        error.message
                    )
                });
            }
        }
    }
    fn expire(&mut self, token: u64) {
        self.pending.retain(|p| p.token != token);
    }
}

#[derive(Clone)]
pub(super) struct NoteDrag {
    pub(super) row: TodayRow,
    pub(super) owner: String,
}
impl Render for NoteDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx);
        div()
            .w(px(220.))
            .h(px(32.))
            .px(px(8.))
            .bg(theme.popover)
            .text_color(theme.foreground)
            .text_size(px(14.))
            .truncate()
            .child(self.row.preview.clone())
    }
}
impl Today {
    #[cfg(test)]
    pub(super) fn archive_busy(&self) -> bool {
        self.model
            .capture()
            .note_actions
            .pending
            .iter()
            .any(|p| matches!(p.action, NoteAction::Archive))
    }
    pub(super) fn note_busy(&self, uuid: &str) -> bool {
        let capture = self.model.capture();
        capture.note_actions.busy(uuid) || capture.appends.pending.iter().any(|p| p.uuid == uuid)
    }
    pub(super) fn drag_eligible(&self, row: &TodayRow, cx: &App) -> bool {
        self.editor.is_none()
            && row.id > 0
            && !row.uuid.is_empty()
            && !row.archived
            && !row.draft
            && !self.note_busy(&row.uuid)
            && self.drag_allowed
            && self.composer.read(cx).value().is_empty()
    }
    pub(super) fn accepts_drop(
        &mut self,
        drag: &NoteDrag,
        target: &Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if drag.owner != self.services.user_id || self.composing(window, cx) {
            return false;
        }
        let Some(row) = self
            .model
            .rows
            .iter()
            .find(|r| r.uuid == drag.row.uuid && r.id == drag.row.id)
        else {
            return false;
        };
        self.drag_eligible(row, cx)
            && match target {
                Destination::Project(id) => self.projects.iter().any(|p| &p.id == id),
                Destination::Archive | Destination::Shared => true,
                Destination::Home => false,
            }
    }
    pub(super) fn drop_note(
        &mut self,
        drag: &NoteDrag,
        destination: &Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_drop(drag, destination, window, cx) {
            return;
        }
        let action = match destination {
            Destination::Project(id) => {
                let project = self
                    .projects
                    .iter()
                    .find(|p| &p.id == id)
                    .expect("checked current project");
                let row = self
                    .model
                    .rows
                    .iter()
                    .find(|r| r.uuid == drag.row.uuid)
                    .expect("checked current row");
                if row.project_id.as_ref() == Some(id) {
                    return;
                }
                NoteAction::Project(project.into())
            }
            Destination::Archive => NoteAction::Archive,
            Destination::Shared => NoteAction::Share,
            Destination::Home => return,
        };
        self.perform_note_action(drag.row.clone(), action, window, cx);
    }
    pub(super) fn selected_action(
        &mut self,
        action: NoteAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcuts_blocked(window, cx) {
            return;
        }
        if let Some(row) = self
            .model
            .selected
            .and_then(|id| self.model.rows.iter().find(|r| r.id == id))
            .cloned()
        {
            self.perform_note_action(row, action, window, cx);
        }
    }
    pub(super) fn perform_note_action(
        &mut self,
        row: TodayRow,
        action: NoteAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcuts_blocked(window, cx)
            || self.note_busy(&row.uuid)
            || row.id <= 0
            || (row.draft && !matches!(action, NoteAction::Restore))
            || row.archived != matches!(action, NoteAction::Restore)
        {
            return;
        }
        let scope = (self.destination.clone(), self.source.human_only);
        let Some(token) =
            self.model
                .capture()
                .note_actions
                .accept(row.clone(), action.clone(), scope)
        else {
            return;
        };
        self.project_notes(window, cx);
        let services = self.services.clone();
        let job = self.services.runtime.spawn(async move {
            let result = match tokio::time::timeout(
                Duration::from_secs(30),
                execute_note_action(&services, &row, action),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(flicknote_client::WireError {
                    code: "timeout".into(),
                    message: "Note action timed out".into(),
                    retryable: false,
                    details: None,
                }),
            };
            services
                .capture
                .lock()
                .expect("capture state")
                .note_actions
                .complete(token, result);
            services.capture_changed.send_replace(());
            // A slow/absent watch cannot keep an assignment/removal projection forever.
            tokio::time::sleep(Duration::from_secs(5)).await;
            services
                .capture
                .lock()
                .expect("capture state")
                .note_actions
                .expire(token);
            services.capture_changed.send_replace(());
        });
        self.services.track(&job);
        cx.notify();
    }
    pub(super) fn project_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self
            .model
            .capture()
            .note_actions
            .project(&self.canonical_rows, &self.destination);
        self.model.snapshot(rows);
        self.refresh_detail(window, cx);
    }
    pub(super) fn related_project(&self) -> Option<&str> {
        let id = self.related_note.or(self.model.selected)?;
        self.model
            .rows
            .iter()
            .find(|r| r.id == id)?
            .project_id
            .as_deref()
    }
}

async fn execute_note_action(
    services: &Services,
    row: &TodayRow,
    action: NoteAction,
) -> Result<Option<String>, flicknote_client::WireError> {
    if let NoteAction::Project(project) = action {
        let local = flicknote_core::backend::LocalPowerSyncBackend::new(
            services.db.clone(),
            services.user_id.clone(),
        );
        services
            .app
            .classify_local_note(&local, &row.uuid, row.id, &project.id)
            .await?;
        return Ok(None);
    }
    // Resolve current canonical lifecycle/identity again before a gateway or lifecycle effect.
    let note = services
        .app
        .handle(AppRequest::NoteGet {
            id: row.id.to_string(),
            archived: row.archived,
        })
        .await?;
    if !matches!(note, AppResponse::NoteDetail(ref note) if note.note.uuid == row.uuid && note.note.short_id == Some(row.id) && (!note.note.draft || row.archived))
    {
        return Err(flicknote_client::WireError {
            code: "invalid_argument".into(),
            message: "Note is no longer eligible for this action".into(),
            retryable: false,
            details: None,
        });
    }
    let request = match action {
        NoteAction::Archive => AppRequest::NoteArchive {
            id: row.id.to_string(),
        },
        NoteAction::Restore => AppRequest::NoteRestore {
            id: row.id.to_string(),
        },
        NoteAction::Share => AppRequest::NoteShare {
            id: row.id.to_string(),
        },
        NoteAction::Unshare => AppRequest::NoteUnshare {
            id: row.id.to_string(),
        },
        NoteAction::Project(_) => unreachable!("classification handled above"),
    };
    match services.app.handle(request).await? {
        AppResponse::Share(result) => Ok(Some(result.url)),
        AppResponse::Unshare(_) | AppResponse::NoteArchive(_) => Ok(None),
        _ => Err(flicknote_client::WireError {
            code: "internal_error".into(),
            message: "Unexpected note action response".into(),
            retryable: false,
            details: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row() -> TodayRow {
        TodayRow {
            id: 7,
            uuid: "original".into(),
            preview: "body".into(),
            content: "body".into(),
            title: None,
            project_id: None,
            project_name: None,
            note_type: "normal".into(),
            project_color: None,
            archived: false,
            draft: false,
            shared: false,
        }
    }
    fn action() -> NoteAction {
        NoteAction::Project(ProjectContextIdentity {
            id: "project".into(),
            name: "Project".into(),
            color: Some("123456".into()),
        })
    }
    #[test]
    fn assignment_watch_ack_order_and_later_writer_never_reapply_overlay() {
        for watch_first in [true, false] {
            let mut state = NoteActions::default();
            let baseline = row();
            let scope = (Destination::Home, false);
            let token = state
                .accept(baseline.clone(), action(), scope.clone())
                .unwrap();
            assert!(
                state
                    .accept(baseline.clone(), NoteAction::Archive, scope.clone())
                    .is_none()
            );
            assert_eq!(
                state.project(std::slice::from_ref(&baseline), &scope.0)[0]
                    .project_color
                    .as_deref(),
                Some("123456")
            );
            let mut watched = baseline.clone();
            watched.project_id = Some("project".into());
            watched.project_color = Some("123456".into());
            if watch_first {
                state.observe(std::slice::from_ref(&watched), &scope);
            }
            state.complete(token, Ok(None));
            if !watch_first {
                state.observe(std::slice::from_ref(&watched), &scope);
            }
            assert!(!state.busy("original"));
            watched.project_id = Some("external".into());
            watched.project_color = Some("abcdef".into());
            assert_eq!(
                state.project(std::slice::from_ref(&watched), &scope.0)[0]
                    .project_color
                    .as_deref(),
                Some("abcdef")
            );
            state.observe(std::slice::from_ref(&baseline), &scope);
            assert_eq!(state.project(&[baseline], &scope.0)[0].project_id, None);
        }
    }
    #[test]
    fn confirmed_share_feedback_yields_to_watch_or_expiry() {
        let baseline = row();
        let scope = (Destination::Home, false);
        for observed in [true, false] {
            let mut state = NoteActions::default();
            let token = state
                .accept(baseline.clone(), NoteAction::Share, scope.clone())
                .unwrap();
            assert!(!state.project(std::slice::from_ref(&baseline), &scope.0)[0].shared);
            state.complete(token, Ok(Some("https://owned.invalid/confirmed".into())));
            assert!(state.busy("original"));
            assert!(state.project(std::slice::from_ref(&baseline), &scope.0)[0].shared);
            assert_eq!(state.links, ["https://owned.invalid/confirmed"]);
            let mut watched = baseline.clone();
            if observed {
                watched.shared = true;
                state.observe(std::slice::from_ref(&watched), &scope);
                watched.shared = false; // later canonical unshare must never reapply confirmation
                state.observe(std::slice::from_ref(&watched), &scope);
            } else {
                state.expire(token);
            }
            assert!(!state.busy("original"));
            assert!(!state.project(&[watched], &scope.0)[0].shared);
        }
    }
    #[test]
    fn removal_scope_expiry_and_errors_keep_watch_authoritative() {
        let baseline = row();
        let scope = (Destination::Shared, false);
        let mut state = NoteActions::default();
        let token = state
            .accept(baseline.clone(), NoteAction::Archive, scope.clone())
            .unwrap();
        assert!(
            state
                .project(std::slice::from_ref(&baseline), &scope.0)
                .is_empty()
        );
        state.observe(&[], &(Destination::Project("different".into()), false));
        assert!(
            state
                .project(std::slice::from_ref(&baseline), &scope.0)
                .is_empty(),
            "another page cannot acknowledge removal"
        );
        state.complete(token, Ok(None));
        state.expire(token);
        assert_eq!(
            state
                .project(std::slice::from_ref(&baseline), &scope.0)
                .len(),
            1,
            "bounded overlay expires"
        );
        for (code, uncertain, action) in [
            ("project_not_found", false, action()),
            ("internal_error", true, action()),
            ("daemon_error", true, action()),
            ("invalid_argument", false, NoteAction::Share),
            ("internal_error", true, NoteAction::Share),
        ] {
            let token = state
                .accept(baseline.clone(), action, scope.clone())
                .unwrap();
            state.complete(
                token,
                Err(flicknote_client::WireError {
                    code: code.into(),
                    message: "owned failure".into(),
                    retryable: false,
                    details: None,
                }),
            );
            assert_eq!(
                state
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("may have completed"),
                uncertain
            );
            assert_eq!(
                state.project(std::slice::from_ref(&baseline), &scope.0)[0].project_id,
                None
            );
            assert!(!state.project(std::slice::from_ref(&baseline), &scope.0)[0].shared);
            assert!(state.links.is_empty());
            assert!(!state.busy("original"));
        }
    }
}
