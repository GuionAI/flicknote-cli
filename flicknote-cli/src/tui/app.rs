use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::types::Note;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    List,
    Detail,
    Search,
}

pub struct App {
    pub view: View,
    pub notes: Vec<Note>,
    pub selected: usize,
    pub search_query: String,
    pub search_input: String,
    pub should_quit: bool,
    db: Database,
}

impl App {
    pub fn new(db: Database) -> Result<Self, CliError> {
        let notes = Self::fetch_notes(&db, None)?;
        Ok(Self {
            view: View::List,
            notes,
            selected: 0,
            search_query: String::new(),
            search_input: String::new(),
            should_quit: false,
            db,
        })
    }

    fn fetch_notes(db: &Database, search: Option<&str>) -> Result<Vec<Note>, CliError> {
        db.read(|conn| {
            let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match search {
                Some(q) if !q.is_empty() => {
                    let escaped = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
                    (
                        "SELECT * FROM notes WHERE deleted_at IS NULL AND title LIKE ? ESCAPE '\\' ORDER BY created_at DESC LIMIT 200".into(),
                        vec![Box::new(format!("%{escaped}%"))],
                    )
                },
                _ => (
                    "SELECT * FROM notes WHERE deleted_at IS NULL ORDER BY created_at DESC LIMIT 200".into(),
                    vec![],
                ),
            };
            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), Note::from_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(CliError::from)
        })
    }

    pub fn selected_note(&self) -> Option<&Note> {
        self.notes.get(self.selected)
    }

    pub fn handle_events(&mut self) -> Result<(), CliError> {
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
                self.view = View::List;
            }
            KeyCode::Enter => {
                self.search_query = self.search_input.clone();
                let search = if self.search_query.is_empty() {
                    None
                } else {
                    Some(self.search_query.as_str())
                };
                self.notes = Self::fetch_notes(&self.db, search)?;
                self.selected = 0;
                self.view = View::List;
            }
            KeyCode::Backspace => {
                self.search_input.pop();
            }
            KeyCode::Char(c) => {
                self.search_input.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn run(mut self, mut terminal: ratatui::DefaultTerminal) -> Result<(), CliError> {
        while !self.should_quit {
            terminal
                .draw(|frame| super::ui::draw(frame, &self))
                .map_err(|e| CliError::Other(e.to_string()))?;
            self.handle_events()?;
        }
        Ok(())
    }
}
