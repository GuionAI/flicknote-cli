use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::fs;

pub(crate) fn run(config: &Config) -> Result<(), CliError> {
    let session_file = &config.paths.session_file;
    if !session_file.exists() {
        println!("Already logged out");
        return Ok(());
    }

    fs::remove_file(session_file)?;
    println!("Logged out");
    Ok(())
}
