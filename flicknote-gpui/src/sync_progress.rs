//! First-sync presentation over one coherent SDK status snapshot.
#[derive(Default)]
pub(crate) struct FirstSync {
    pub(crate) complete: bool,
    observed: bool,
    pub(crate) progress: Option<Progress>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Progress {
    Indeterminate,
    Percent(u8),
}

#[derive(Clone, Copy)]
pub(crate) struct Snapshot {
    pub(crate) connected: bool,
    pub(crate) connecting: bool,
    pub(crate) downloading: bool,
    pub(crate) error: bool,
    // None means the required active default subscriptions are not known yet.
    pub(crate) required_ready: Option<bool>,
    pub(crate) notes_applied: bool,
    pub(crate) notes_progress: Option<(i64, i64)>,
}

impl FirstSync {
    pub(crate) fn update(&mut self, status: Snapshot) -> Option<String> {
        // A cached completed first sync needs no initial indicator, including
        // offline startup. Later offline/error events cannot establish readiness.
        if status.required_ready == Some(true)
            && !status.error
            && (status.connected || !self.observed)
        {
            self.complete = true;
        }
        self.observed = true;
        self.progress = None;
        if status.error {
            return Some(
                "Sync unavailable. Cached notes remain available; new notes need a connection."
                    .into(),
            );
        }
        if !status.connected && !status.connecting {
            return Some("Offline. Showing cached notes.".into());
        }
        if self.complete {
            return (status.connecting || status.downloading).then(|| "Syncing…".into());
        }
        if status.notes_applied {
            self.progress = Some(Progress::Percent(90));
            return Some("First sync: 90% — Finishing sync…".into());
        }
        if let Some((total, downloaded)) = status.notes_progress.filter(|(total, _)| *total > 0) {
            let percent = ((downloaded.max(0) as f64 / total as f64).min(1.) * 90.) as u8;
            self.progress = Some(Progress::Percent(percent));
            return Some(if downloaded >= total {
                format!("First sync: {percent}% — Applying notes…")
            } else {
                format!("First sync: {percent}% — Downloading notes…")
            });
        }
        self.progress = Some(Progress::Indeterminate);
        Some(if status.connecting {
            "First sync: Connecting…".into()
        } else {
            "First sync: Syncing…".into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            connected: true,
            connecting: false,
            downloading: true,
            error: false,
            required_ready: Some(false),
            notes_applied: false,
            notes_progress: None,
        }
    }

    #[test]
    fn notes_download_is_weighted_and_application_is_a_separate_checkpoint() {
        let mut first = FirstSync::default();
        let mut s = snapshot();
        s.notes_progress = Some((100, 50));
        assert_eq!(
            first.update(s).as_deref(),
            Some("First sync: 45% — Downloading notes…")
        );
        let mut s = snapshot();
        s.notes_progress = Some((100, 100));
        assert_eq!(
            first.update(s).as_deref(),
            Some("First sync: 90% — Applying notes…")
        );
        let mut s = snapshot();
        s.notes_applied = true;
        assert_eq!(
            first.update(s).as_deref(),
            Some("First sync: 90% — Finishing sync…")
        );
        assert!(!first.complete);
        let mut s = snapshot();
        s.required_ready = Some(true);
        s.downloading = false;
        assert_eq!(first.update(s), None);
        assert_eq!(first.progress, None);
        assert!(first.complete);
    }

    #[test]
    fn unknown_and_zero_totals_are_indeterminate() {
        for counters in [None, Some((0, 0)), Some((-1, 5))] {
            let mut first = FirstSync::default();
            let mut s = snapshot();
            s.notes_progress = counters;
            s.required_ready = None;
            assert_eq!(first.update(s).as_deref(), Some("First sync: Syncing…"));
            assert!(!first.complete);
            assert_eq!(first.progress, Some(Progress::Indeterminate));
        }
        let mut first = FirstSync::default();
        let mut s = snapshot();
        s.connected = false;
        s.connecting = true;
        assert_eq!(first.update(s).as_deref(), Some("First sync: Connecting…"));
    }

    #[test]
    fn cached_completion_and_reconnect_do_not_restart_first_sync() {
        let mut first = FirstSync::default();
        let mut s = snapshot();
        s.connected = false;
        s.downloading = false;
        s.required_ready = Some(true);
        assert_eq!(
            first.update(s).as_deref(),
            Some("Offline. Showing cached notes.")
        );
        assert!(first.complete);
        let mut s = snapshot();
        s.required_ready = None;
        s.connected = false;
        s.connecting = true;
        s.downloading = false;
        assert_eq!(first.update(s).as_deref(), Some("Syncing…"));
        assert!(first.complete);
        assert_eq!(first.progress, None);
        s.connected = true;
        s.connecting = false;
        assert_eq!(first.update(s), None);
        assert_eq!(first.progress, None);
    }

    #[test]
    fn offline_or_error_cannot_complete_an_in_progress_first_sync() {
        for error in [false, true] {
            let mut first = FirstSync::default();
            first.update(snapshot());
            let mut s = snapshot();
            s.connected = false;
            s.error = error;
            s.required_ready = Some(true);
            assert_eq!(
                first.update(s).as_deref(),
                Some(if error {
                    "Sync unavailable. Cached notes remain available; new notes need a connection."
                } else {
                    "Offline. Showing cached notes."
                })
            );
            assert!(!first.complete);
            assert_eq!(first.progress, None);
        }
    }

    #[test]
    fn invalid_download_counters_stay_within_the_notes_weight() {
        for (downloaded, expected) in [
            (-1, "First sync: 0% — Downloading notes…"),
            (1000, "First sync: 90% — Applying notes…"),
        ] {
            let mut first = FirstSync::default();
            let mut s = snapshot();
            s.notes_progress = Some((100, downloaded));
            assert_eq!(first.update(s).as_deref(), Some(expected));
            assert!(!first.complete);
        }
    }
}
