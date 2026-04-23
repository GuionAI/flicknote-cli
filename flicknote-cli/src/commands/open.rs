use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

#[derive(Args)]
pub(crate) struct OpenArgs {
    /// Note ID (full UUID or prefix)
    id: String,
}

pub(crate) fn run(db: &dyn NoteDb, config: &Config, args: &OpenArgs) -> Result<(), CliError> {
    let web_url = config.web_url.as_deref().ok_or_else(|| {
        CliError::Other(
            "webUrl not configured. Set it in ~/.config/flicknote/config.json or FLICKNOTE_WEB_URL."
                .into(),
        )
    })?;
    let full_id = db.resolve_note_id(&args.id)?;
    let url = format!("{}/notes/{}", web_url.trim_end_matches('/'), full_id);
    open::that(&url).map_err(CliError::Io)?;
    println!("Opened {} — {}", full_id, url);
    Ok(())
}
