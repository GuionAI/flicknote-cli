use crate::tui::app::App;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::panic;

pub(crate) fn run(_config: &Config, db: &dyn NoteDb) -> Result<(), CliError> {
    let app = App::new(db)?;

    let terminal = ratatui::init();

    // Restore terminal on panic so the shell isn't left in raw mode
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        default_hook(info);
    }));

    let result = app.run(terminal);
    ratatui::restore();

    result
}
