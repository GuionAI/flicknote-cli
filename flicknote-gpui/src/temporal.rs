//! Process-local Home and per-project calendar choices, with no saved preferences.
use super::*;
use flicknote_sync::today::{Period, Range};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct Memory {
    home: Option<chrono::NaiveDate>,
    pub(super) search_return: Option<super::search::RetainedOrigin>,
    projects: HashMap<String, (bool, Option<chrono::NaiveDate>)>,
}
impl Memory {
    pub(super) fn period(&self, destination: &Destination) -> Period {
        match destination {
            Destination::Home => Period::Day(self.home),
            Destination::Project(id) => match self.projects.get(id) {
                Some((true, week)) => Period::Week(*week),
                _ => Period::All,
            },
            _ => Period::All,
        }
    }
    pub(super) fn reset_home(&mut self) {
        self.home = None;
    }
    pub(super) fn projects_remove(&mut self, id: &str) {
        self.projects.remove(id);
    }
    fn save(&mut self, destination: &Destination, period: &Period) {
        match (destination, period) {
            (Destination::Home, Period::Day(day)) => self.home = *day,
            (Destination::Project(id), Period::Week(week)) => {
                self.projects.insert(id.clone(), (true, *week));
            }
            (Destination::Project(id), Period::All) => {
                self.projects.entry(id.clone()).or_default().0 = false;
            }
            _ => {}
        }
    }
}
impl Today {
    pub(super) fn action_scope(&self) -> (Destination, bool, Option<Range>, Option<String>) {
        if self.search.active() {
            (
                Destination::Home,
                self.effective_human_only(),
                None,
                Some(self.search.query.clone()),
            )
        } else {
            (
                self.destination.clone(),
                self.effective_human_only(),
                self.range,
                None,
            )
        }
    }
    pub(super) fn pending_visible(&self, pending: &crate::model::Pending) -> bool {
        !self.search.active()
            && self.destination == Destination::Home
            && matches!(self.period, Period::Day(None))
            && self.range.is_some_and(|(start, end)| {
                pending.accepted_at >= start && pending.accepted_at < end
            })
    }
    fn set_period(&mut self, period: Period, window: &mut Window, cx: &mut Context<Self>) {
        if period == self.period {
            return;
        }
        self.services
            .temporal
            .lock()
            .expect("calendar memory")
            .save(&self.destination, &period);
        self.period = period;
        self.reset_projection(window, cx);
    }
    pub(super) fn move_period(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.active()
            || self.search_focused(window, cx)
            || self.shortcuts_blocked(window, cx)
        {
            return;
        }
        if let Some(period) = self.period.shifted(next, &chrono::Local::now()) {
            self.set_period(period, window, cx);
        }
    }
    pub(super) fn mouse_period(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() || self.composing(window, cx) {
            return;
        }
        if let Some(period) = self.period.shifted(next, &chrono::Local::now()) {
            self.set_period(period, window, cx);
        }
    }
    pub(super) fn current_period(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() || self.composing(window, cx) {
            return;
        }
        match self.period {
            Period::Day(_) => self.set_period(Period::Day(None), window, cx),
            Period::Week(_) => self.set_period(Period::Week(None), window, cx),
            Period::All => {}
        }
    }
    pub(super) fn project_scope(
        &mut self,
        week: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.is_some() || self.composing(window, cx) {
            return;
        }
        if let Destination::Project(id) = &self.destination {
            let period = if week {
                let memory = self.services.temporal.lock().expect("calendar memory");
                Period::Week(memory.projects.get(id).and_then(|p| p.1))
            } else {
                Period::All
            };
            self.set_period(period, window, cx);
        }
    }
    pub(super) fn home_today(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() || self.composing(window, cx) {
            return;
        }
        if self.search.active() {
            self.exit_search(false, window, cx);
        }
        self.services.temporal.lock().expect("calendar memory").home = None;
        if self.destination == Destination::Home {
            self.set_period(Period::Day(None), window, cx);
            self.close_detail(window, cx);
        } else {
            self.set_destination(Destination::Home, window, cx);
        }
    }
    pub(super) fn period_label(&self) -> String {
        match self.period {
            Period::All => String::new(),
            Period::Day(None) => "Today".into(),
            Period::Day(Some(day)) => day.format("%b %-d, %Y").to_string(),
            Period::Week(_) => {
                let day = self
                    .range
                    .map(|r| r.0.with_timezone(&chrono::Local).date_naive())
                    .unwrap_or_else(|| self.period.date(&chrono::Local::now()));
                let end = day
                    .checked_add_days(chrono::Days::new(6))
                    .expect("calendar week");
                format!("{} – {}", day.format("%b %-d"), end.format("%b %-d, %Y"))
            }
        }
    }
}
