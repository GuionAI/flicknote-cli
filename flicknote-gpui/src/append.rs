//! Process-local append identity and optimistic overlay; never a stored document.
use flicknote_client::{AppResponse, WireError};
use flicknote_sync::today::TodayRow;

#[derive(Default)]
pub(crate) struct Appends {
    pub(crate) pending: Vec<Append>,
    pub(crate) recovery: Vec<AppendRecovery>,
    next_token: u64,
}
pub(crate) struct Append {
    pub(crate) token: u64,
    pub(crate) uuid: String,
    pub(crate) id: i64,
    pub(crate) text: String,
    baseline: String,
    pub(crate) expected: String,
    acknowledged: bool,
    observed: bool,
}
pub(crate) struct AppendRecovery {
    pub(crate) token: u64,
    pub(crate) uuid: String,
    pub(crate) id: i64,
    pub(crate) text: String,
    pub(crate) uncertain: bool,
    pub(crate) message: String,
    pub(crate) restore: bool,
}
impl Appends {
    pub(crate) fn accept(&mut self, row: &TodayRow, text: String) -> Option<u64> {
        if self.pending.iter().any(|p| p.uuid == row.uuid) {
            return None;
        }
        self.next_token += 1;
        let expected = if row.content.is_empty() {
            text.clone()
        } else {
            format!("{}\n\n{text}", row.content)
        };
        self.pending.push(Append {
            token: self.next_token,
            uuid: row.uuid.clone(),
            id: row.id,
            text,
            baseline: row.content.clone(),
            expected,
            acknowledged: false,
            observed: false,
        });
        Some(self.next_token)
    }
    pub(crate) fn observe(&mut self, rows: &[TodayRow]) {
        for pending in &mut self.pending {
            if let Some(row) = rows.iter().find(|r| r.uuid == pending.uuid)
                && row.content != pending.baseline
            {
                // Matching append or an unrelated writer: canonical watch is authoritative.
                pending.observed = true;
            }
        }
        self.pending.retain(|p| !p.acknowledged || !p.observed);
    }
    pub(crate) fn source(&self, row: &TodayRow) -> String {
        self.pending
            .iter()
            .find(|p| p.uuid == row.uuid && !p.observed && row.content == p.baseline)
            .map_or_else(|| row.content.clone(), |p| p.expected.clone())
    }
    pub(crate) fn restore(&mut self, target: Option<&str>, eligible: bool) -> Option<String> {
        let mut text = None;
        self.recovery.retain_mut(|recovery| {
            let restore = std::mem::take(&mut recovery.restore);
            if restore && eligible && target == Some(recovery.uuid.as_str()) && text.is_none() {
                text = Some(recovery.text.clone());
                false
            } else {
                true
            }
        });
        text
    }
    pub(crate) fn complete(&mut self, token: u64, result: Result<AppResponse, WireError>) {
        let Some(index) = self.pending.iter().position(|p| p.token == token) else {
            return;
        };
        if matches!(result, Ok(AppResponse::NoteMutation(_))) {
            self.pending[index].acknowledged = true;
            if self.pending[index].observed {
                self.pending.remove(index);
            }
            return;
        }
        let pending = self.pending.remove(index);
        // Only argument validation is proven pre-write at the existing service boundary.
        // Even note_not_found can come from mutation_result AFTER update_note_content.
        let (uncertain, message) = match result {
            Err(error) => (error.code != "invalid_argument", error.message),
            Ok(_) => (true, "Unexpected append response".into()),
        };
        self.recovery.push(AppendRecovery {
            token,
            uuid: pending.uuid,
            id: pending.id,
            text: pending.text,
            uncertain,
            message,
            restore: !uncertain,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(content: &str) -> TodayRow {
        TodayRow {
            id: 7,
            uuid: "original".into(),
            content: content.into(),
            preview: String::new(),
            title: Some("title".into()),
            project_id: None,
            project_name: None,
            project_color: None,
            archived: false,
            draft: false,
            shared: false,
            failed_stage: None,
            note_type: "normal".into(),
        }
    }
    fn success() -> AppResponse {
        AppResponse::NoteMutation(serde_json::from_value(serde_json::json!({
            "note": {"id":7,"uuid":"original","type":"normal","topics":[],"content_bytes":12,"flagged":false,"draft":false}, "sections":[]
        })).unwrap())
    }
    fn rejection() -> WireError {
        WireError {
            code: "invalid_argument".into(),
            message: "Rejected".into(),
            details: None,
            retryable: false,
        }
    }
    #[test]
    fn append_ack_watch_orders_and_external_writer_are_authoritative() {
        for watch_first in [true, false] {
            let mut appends = Appends::default();
            let baseline = row("body");
            let token = appends.accept(&baseline, "  text\n".into()).unwrap();
            assert_eq!(appends.source(&baseline), "body\n\n  text\n");
            assert!(appends.accept(&baseline, "second".into()).is_none());
            let changed = row("body\n\n  text\n");
            if watch_first {
                appends.observe(std::slice::from_ref(&changed));
            }
            appends.complete(token, Ok(success()));
            if !watch_first {
                appends.observe(std::slice::from_ref(&changed));
            }
            assert!(appends.pending.is_empty());
            assert_eq!(appends.source(&changed), changed.content);
        }
        let mut appends = Appends::default();
        let token = appends.accept(&row("body"), "addition".into()).unwrap();
        let changed = row("unrelated writer");
        appends.observe(std::slice::from_ref(&changed));
        assert_eq!(appends.source(&changed), "unrelated writer");
        appends.complete(token, Ok(success()));
        assert!(appends.pending.is_empty());
    }
    #[test]
    fn append_rejection_removes_only_its_overlay_and_restores_only_matching_empty_context() {
        for (target, eligible, restore) in [
            (Some("original"), true, true),
            (Some("other"), true, false),
            (Some("original"), false, false),
            (None, true, false),
        ] {
            let mut appends = Appends::default();
            let baseline = row("body");
            let token = appends.accept(&baseline, "recover".into()).unwrap();
            let mut other = row("other body");
            other.uuid = "other".into();
            appends.accept(&other, "other addition".into()).unwrap();
            appends.complete(token, Err(rejection()));
            assert_eq!(appends.source(&baseline), "body");
            assert_eq!(appends.source(&other), "other body\n\nother addition");
            assert_eq!(
                appends.restore(target, eligible),
                restore.then(|| "recover".into())
            );
            if !restore {
                assert_eq!(appends.recovery[0].text, "recover");
            }
            assert_eq!(
                appends.restore(Some("original"), true),
                None,
                "later typing/context changes never trigger stale auto-restore"
            );
        }
    }
}
