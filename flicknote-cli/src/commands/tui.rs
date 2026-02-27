use crate::tui::app::App;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use std::panic;

pub fn run(config: &Config) -> Result<(), CliError> {
    let db = Database::open_local(config)?;
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
