use std::cell::Cell;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use flicknote_core::backend::{NoteDb, NoteFilter};
use flicknote_core::error::CliError;
use flicknote_core::types::{Note, Project};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum View {
    List,
    Detail,
    Search,
}

pub(crate) struct App<'a> {
    pub view: View,
    pub notes: Vec<Note>,
    pub selected: usize,
    pub search_query: String,
    pub search_input: String,
    pub should_quit: bool,
    pub projects: Vec<Project>,
    pub scroll_offset: u16,
    pub detail_content_height: Cell<u16>,
    pub detail_visible_height: Cell<u16>,
    pub autocomplete_matches: Vec<String>,
    pub autocomplete_index: usize,
    db: &'a dyn NoteDb,
}

impl<'a> App<'a> {
    pub(crate) fn new(db: &'a dyn NoteDb) -> Result<Self, CliError> {
        let notes = Self::fetch_notes(db, None)?;
        let projects = Self::fetch_projects(db)?;
        Ok(Self {
            view: View::List,
            notes,
            selected: 0,
            search_query: String::new(),
            search_input: String::new(),
            scroll_offset: 0,
            detail_content_height: Cell::new(0),
            detail_visible_height: Cell::new(0),
            should_quit: false,
            projects,
            autocomplete_matches: Vec::new(),
            autocomplete_index: 0,
            db,
        })
    }

    fn fetch_notes(db: &dyn NoteDb, search: Option<&str>) -> Result<Vec<Note>, CliError> {
        let filter = NoteFilter {
            project_id: None,
            note_type: None,
            archived: false,
            limit: 200,
        };
        match search {
            Some(q) if !q.is_empty() => db.search_notes(&[q.to_string()], &filter),
            _ => db.list_notes(&filter),
        }
    }

    fn fetch_projects(db: &dyn NoteDb) -> Result<Vec<Project>, CliError> {
        db.list_projects(false)
    }

    fn update_autocomplete(&mut self) {
        if let Some(prefix) = self.search_input.strip_prefix("project:") {
            let prefix_lower = prefix.to_lowercase();
            self.autocomplete_matches = self
                .projects
                .iter()
                .filter(|p| p.name.to_lowercase().starts_with(&prefix_lower))
                .map(|p| p.name.clone())
                .collect();
            self.autocomplete_index = 0;
        } else {
            self.autocomplete_matches.clear();
        }
    }

    pub(crate) fn selected_note(&self) -> Option<&Note> {
        self.notes.get(self.selected)
    }

    fn search_filter(&self) -> Option<&str> {
        if self.search_query.is_empty() {
            None
        } else {
            Some(self.search_query.as_str())
        }
    }

    pub(crate) fn handle_events(&mut self) -> Result<(), CliError> {
        let Event::Key(key) = event::read().map_err(|e| CliError::Other(e.to_string()))? else {
            return Ok(());
        };
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match self.view {
            View::List => self.handle_list_key(key.code),
            View::Detail => self.handle_detail_key(key),
            View::Search => self.handle_search_key(key.code)?,
        }
        Ok(())
    }

    fn handle_list_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.notes.is_empty() {
                    self.selected = (self.selected + 1).min(self.notes.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Char('g') | KeyCode::Home => self.selected = 0,
            KeyCode::Char('G') | KeyCode::End => {
                if !self.notes.is_empty() {
                    self.selected = self.notes.len() - 1;
                }
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                if !self.notes.is_empty() {
                    self.scroll_offset = 0;
                    self.view = View::Detail;
                }
            }
            KeyCode::Char('/') => {
                self.search_input = self.search_query.clone();
                self.view = View::Search;
            }
            KeyCode::Char('d') => {
                if let Some(note) = self.notes.get(self.selected) {
                    let id = note.id.clone();
                    let now = chrono::Utc::now().to_rfc3339();
                    let result = self.db.set_note_deleted_at(&id, Some(&now));
                    if result.is_ok() {
                        self.notes.remove(self.selected);
                        if self.selected >= self.notes.len() && !self.notes.is_empty() {
                            self.selected = self.notes.len() - 1;
                        }
                    }
                }
            }
            KeyCode::Char('r') => {
                if let Ok(notes) = Self::fetch_notes(self.db, self.search_filter()) {
                    self.notes = notes;
                    if self.selected >= self.notes.len() {
                        self.selected = self.notes.len().saturating_sub(1);
                    }
                }
                self.projects = Self::fetch_projects(self.db).unwrap_or_default();
            }
            KeyCode::Char('u') => {
                let result = self.db.undo_last_delete();
                if result.is_ok()
                    && let Ok(notes) = Self::fetch_notes(self.db, self.search_filter())
                {
                    self.notes = notes;
                }
            }
            _ => {}
        }
    }

    fn handle_detail_key(&mut self, key: KeyEvent) {
        let max = self.detail_content_height.get();
        let half_page = self.detail_visible_height.get() / 2;

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _)
            | (KeyCode::Char('q'), KeyModifiers::NONE)
            | (KeyCode::Char('h'), KeyModifiers::NONE)
            | (KeyCode::Left, _) => {
                self.view = View::List;
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                if self.scroll_offset < max {
                    self.scroll_offset = self.scroll_offset.saturating_add(1);
                }
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                self.scroll_offset = (self.scroll_offset + half_page).min(max);
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(half_page);
            }
            (KeyCode::Char('g'), KeyModifiers::NONE) | (KeyCode::Home, _) => {
                self.scroll_offset = 0;
            }
            (KeyCode::Char('G'), KeyModifiers::SHIFT | KeyModifiers::NONE) | (KeyCode::End, _) => {
                self.scroll_offset = max;
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyCode) -> Result<(), CliError> {
        match key {
            KeyCode::Esc => {
                self.autocomplete_matches.clear();
                self.view = View::List;
            }
            KeyCode::Enter => {
                self.search_query = self.search_input.clone();
                self.autocomplete_matches.clear();
                if let Some(project_name) = self.search_query.strip_prefix("project:") {
                    let project_name = project_name.trim();
                    let Some(project_id) = self.db.find_project_by_name(project_name)? else {
                        self.notes = vec![];
                        self.selected = 0;
                        self.view = View::List;
                        return Ok(());
                    };
                    let filter = NoteFilter {
                        project_id: Some(project_id.as_str()),
                        note_type: None,
                        archived: false,
                        limit: 200,
                    };
                    self.notes = self.db.list_notes(&filter)?;
                } else {
                    self.notes = Self::fetch_notes(self.db, self.search_filter())?;
                }
                self.selected = 0;
                self.view = View::List;
            }
            KeyCode::Tab => {
                if !self.autocomplete_matches.is_empty() {
                    let name = self.autocomplete_matches[self.autocomplete_index].clone();
                    self.search_input = format!("project:{name}");
                    self.autocomplete_index =
                        (self.autocomplete_index + 1) % self.autocomplete_matches.len();
                }
            }
            KeyCode::Backspace => {
                self.search_input.pop();
                self.update_autocomplete();
            }
            KeyCode::Char(c) => {
                self.search_input.push(c);
                self.update_autocomplete();
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn run(mut self, mut terminal: ratatui::DefaultTerminal) -> Result<(), CliError> {
        while !self.should_quit {
            terminal
                .draw(|frame| super::ui::draw(frame, &self))
                .map_err(|e| CliError::Other(e.to_string()))?;
            self.handle_events()?;
        }
        Ok(())
    }
}
