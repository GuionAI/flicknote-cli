use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::types::{Note, Project};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum View {
    List,
    Detail,
    Search,
}

pub(crate) struct App {
    pub view: View,
    pub notes: Vec<Note>,
    pub selected: usize,
    pub search_query: String,
    pub search_input: String,
    pub should_quit: bool,
    pub projects: Vec<Project>,
    pub autocomplete_matches: Vec<String>,
    pub autocomplete_index: usize,
    db: Database,
}

impl App {
    pub(crate) fn new(db: Database) -> Result<Self, CliError> {
        let notes = Self::fetch_notes(&db, None)?;
        let projects = Self::fetch_projects(&db)?;
        Ok(Self {
            view: View::List,
            notes,
            selected: 0,
            search_query: String::new(),
            search_input: String::new(),
            should_quit: false,
            projects,
            autocomplete_matches: Vec::new(),
            autocomplete_index: 0,
            db,
        })
    }

    fn fetch_notes(db: &Database, search: Option<&str>) -> Result<Vec<Note>, CliError> {
        db.read(|conn| {
            let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match search {
                Some(q) if !q.is_empty() => {
                    let escaped = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
                    {
                        let pattern = format!("%{escaped}%");
                        (
                            "SELECT * FROM notes WHERE deleted_at IS NULL AND (title LIKE ? ESCAPE '\\' OR content LIKE ? ESCAPE '\\') ORDER BY created_at DESC LIMIT 200".into(),
                            vec![Box::new(pattern.clone()), Box::new(pattern)],
                        )
                    }
                },
                _ => (
                    "SELECT * FROM notes WHERE deleted_at IS NULL ORDER BY created_at DESC LIMIT 200".into(),
                    vec![],
                ),
            };
            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(std::convert::AsRef::as_ref).collect();
            let rows = stmt.query_map(param_refs.as_slice(), Note::from_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(CliError::from)
        })
    }

    fn fetch_projects(db: &Database) -> Result<Vec<Project>, CliError> {
        db.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT * FROM projects WHERE is_archived = 0 OR is_archived IS NULL ORDER BY name",
            )?;
            let rows = stmt.query_map([], Project::from_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(CliError::from)
        })
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

    pub(crate) fn handle_events(&mut self) -> Result<(), CliError> {
        let Event::Key(key) = event::read().map_err(|e| CliError::Other(e.to_string()))? else {
            return Ok(());
        };
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match self.view {
            View::List => self.handle_list_key(key.code),
            View::Detail => self.handle_detail_key(key.code),
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
                    let result = self.db.write(|conn| {
                        conn.execute(
                            "UPDATE notes SET deleted_at = ?, updated_at = ? WHERE id = ?",
                            rusqlite::params![&now, &now, &id],
                        )?;
                        Ok(())
                    });
                    if result.is_ok() {
                        self.notes.remove(self.selected);
                        if self.selected >= self.notes.len() && !self.notes.is_empty() {
                            self.selected = self.notes.len() - 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_detail_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') | KeyCode::Left => {
                self.view = View::List;
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
                    let project_name = project_name.trim().to_string();
                    self.notes = self.db.read(|conn| {
                        let mut stmt = conn.prepare(
                            "SELECT n.* FROM notes n \
                             JOIN projects p ON n.project_id = p.id \
                             WHERE p.name = ? COLLATE NOCASE AND n.deleted_at IS NULL \
                             ORDER BY n.created_at DESC LIMIT 200",
                        )?;
                        let rows =
                            stmt.query_map(rusqlite::params![project_name], Note::from_row)?;
                        rows.collect::<Result<Vec<_>, _>>().map_err(CliError::from)
                    })?;
                } else {
                    let search = if self.search_query.is_empty() {
                        None
                    } else {
                        Some(self.search_query.as_str())
                    };
                    self.notes = Self::fetch_notes(&self.db, search)?;
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
