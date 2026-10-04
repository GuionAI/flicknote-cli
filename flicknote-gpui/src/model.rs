use flicknote_sync::spike::today::TodayRow;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug)]
pub(crate) struct Pending {
    pub(crate) token: u64,
    pub(crate) text: String,
    pub(crate) id: Option<i64>,
}
#[derive(Default)]
pub(crate) struct Capture {
    pub(crate) pending: Vec<Pending>,
    pub(crate) recovery: Vec<String>,
    pub(crate) uncertain: Vec<(String, flicknote_client::WireError)>,
    pub(crate) error: Option<String>,
    pub(crate) restore: bool,
    next_token: u64,
}
#[derive(Default)]
pub(crate) struct Model {
    pub(crate) rows: Arc<Vec<TodayRow>>,
    pub(crate) selected: Option<i64>,
    pub(crate) capture: Arc<Mutex<Capture>>,
}
impl Model {
    pub(crate) fn capture(&self) -> MutexGuard<'_, Capture> {
        self.capture.lock().expect("capture state")
    }
    pub(crate) fn accept(&mut self, text: String) -> u64 {
        self.capture().accept(text)
    }
    pub(crate) fn snapshot(&mut self, rows: Arc<Vec<TodayRow>>) {
        if let Some(id) = self.selected
            && !rows.iter().any(|r| r.id == id)
        {
            let surviving = rows
                .iter()
                .map(|r| r.id)
                .collect::<std::collections::HashSet<_>>();
            self.selected = self
                .rows
                .iter()
                .position(|r| r.id == id)
                .and_then(|index| {
                    self.rows[index + 1..]
                        .iter()
                        .chain(self.rows[..index].iter().rev())
                        .find(|row| surviving.contains(&row.id))
                        .map(|row| row.id)
                })
                .or_else(|| rows.first().map(|row| row.id));
        }
        self.rows = rows;
        self.reconcile();
    }
    #[cfg(test)]
    pub(crate) fn ack(&mut self, token: u64, result: Result<i64, String>) -> Option<String> {
        let error = self.capture().ack(token, result);
        self.reconcile();
        error
    }
    #[cfg(test)]
    pub(crate) fn uncertain(&mut self, token: u64, error: flicknote_client::WireError) {
        self.capture().uncertain(token, error);
        self.reconcile();
    }
    pub(crate) fn reconcile(&mut self) {
        self.capture()
            .pending
            .retain(|p| !p.id.is_some_and(|id| self.rows.iter().any(|r| r.id == id)));
    }
    pub(crate) fn archive_success(&mut self, id: i64) {
        self.snapshot(Arc::new(
            self.rows.iter().filter(|r| r.id != id).cloned().collect(),
        ));
    }
}

impl Capture {
    pub(crate) fn accept(&mut self, text: String) -> u64 {
        self.next_token += 1;
        self.pending.push(Pending {
            token: self.next_token,
            text,
            id: None,
        });
        self.next_token
    }
    pub(crate) fn complete(
        &mut self,
        token: u64,
        result: Result<flicknote_client::AppResponse, flicknote_client::WireError>,
        restore: bool,
    ) {
        use flicknote_client::AppResponse;
        let result = match result {
            Ok(AppResponse::NoteCreate(note)) => Ok(note.id),
            Ok(_) => Err("Unexpected create response".into()),
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "note_create_unknown" | "note_create_partial"
                ) =>
            {
                self.error = Some(format!("{} Do not submit this note again.", error.message));
                self.uncertain(token, error);
                return;
            }
            Err(error) => Err(error.message),
        };
        if let Some(error) = self.ack(token, result) {
            self.error = Some(format!("Could not save: {error}"));
            self.restore = restore;
        }
    }
    pub(crate) fn ack(&mut self, token: u64, result: Result<i64, String>) -> Option<String> {
        let index = self.pending.iter().position(|p| p.token == token)?;
        match result {
            Ok(id) => {
                self.pending[index].id = Some(id);
                None
            }
            Err(error) => {
                self.recovery.push(self.pending.remove(index).text);
                Some(error)
            }
        }
    }
    pub(crate) fn uncertain(&mut self, token: u64, error: flicknote_client::WireError) {
        if let Some(index) = self.pending.iter().position(|p| p.token == token) {
            let pending = self.pending.remove(index);
            let text = pending.text.clone();
            if let Some(id) = error.details.as_ref().and_then(|d| d["short_id"].as_i64()) {
                self.pending.push(Pending {
                    id: Some(id),
                    ..pending
                });
            }
            self.uncertain.push((text, error));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(ids: &[i64]) -> Arc<Vec<TodayRow>> {
        Arc::new(
            ids.iter()
                .map(|id| TodayRow {
                    id: *id,
                    uuid: format!("uuid-{id}"),
                    preview: format!("Note {id}"),
                    content: format!("Content {id}"),
                    note_type: "normal".into(),
                    project_color: None,
                })
                .collect(),
        )
    }
    #[test]
    fn both_ack_watch_orders_reconcile_once_by_canonical_identity() {
        for watch_first in [true, false] {
            let mut model = Model::default();
            let token = model.accept("input".into());
            assert!(model.selected.is_none());
            if watch_first {
                model.snapshot(rows(&[7, 5]));
            }
            assert!(model.ack(token, Ok(7)).is_none());
            if !watch_first {
                assert_eq!(model.capture().pending.len(), 1);
                model.snapshot(rows(&[7, 5]));
            }
            assert!(model.capture().pending.is_empty());
            model.snapshot(rows(&[7, 5]));
            assert_eq!(model.rows.len(), 2);
        }
    }
    #[test]
    fn uncertain_results_keep_identity_without_safe_resubmission_recovery() {
        for (code, id) in [
            ("note_create_unknown", None),
            ("note_create_partial", Some(80)),
        ] {
            let mut model = Model::default();
            let token = model.accept("original".into());
            let error = flicknote_client::WireError {
                code: code.into(),
                message: "Do not create it again".into(),
                retryable: false,
                details: Some(serde_json::json!({"note_id":"stable-uuid","short_id":id})),
            };
            model.uncertain(token, error.clone());
            assert!(model.capture().recovery.is_empty());
            assert_eq!(model.capture().uncertain[0].1, error);
            if id.is_some() {
                model.snapshot(rows(&[80]));
            }
            assert!(model.capture().pending.is_empty());
        }
    }
    #[test]
    fn failure_recovers_input_without_retargeting_or_mutation_retry() {
        let mut model = Model::default();
        model.snapshot(rows(&[5, 4, 3]));
        model.selected = Some(4);
        let token = model.accept("unsaved".into());
        assert_eq!(
            model.ack(token, Err("failure".into())),
            Some("failure".into())
        );
        assert_eq!(model.capture().recovery, ["unsaved"]);
        assert!(model.capture().pending.is_empty());
        assert_eq!(model.selected, Some(4));
        assert_eq!(model.rows.len(), 3);
        model.archive_success(4);
        assert_eq!(model.selected, Some(3));
        model.archive_success(3);
        assert_eq!(model.selected, Some(5));
        model.archive_success(5);
        assert!(model.selected.is_none());
    }
    #[test]
    fn external_removal_and_reorder_preserve_selection_by_id() {
        let mut model = Model::default();
        model.snapshot(rows(&[5, 4, 3]));
        model.selected = Some(4);
        model.snapshot(rows(&[7, 5, 4, 3]));
        assert_eq!(model.selected, Some(4));
        model.snapshot(rows(&[7, 5, 3]));
        assert_eq!(model.selected, Some(3));
    }
    #[test]
    fn batched_insert_and_archive_follow_surviving_neighbours_in_both_orders() {
        for (old, changed, expected) in [
            (&[5, 4, 3][..], &[6, 5, 3][..], Some(3)),
            (&[7, 5, 4, 3, 2][..], &[8, 7, 5, 2][..], Some(2)),
            (&[5, 4, 3][..], &[6, 5][..], Some(5)),
            (&[5, 4, 3][..], &[6][..], Some(6)),
            (&[5, 4, 3][..], &[][..], None),
        ] {
            for watch_first in [true, false] {
                let mut model = Model::default();
                model.snapshot(rows(old));
                model.selected = Some(4);
                if watch_first {
                    model.snapshot(rows(changed));
                    assert_eq!(model.selected, expected);
                }
                model.archive_success(4);
                if !watch_first {
                    model.snapshot(rows(changed));
                }
                assert_eq!(model.selected, expected);
            }
        }
    }
}
