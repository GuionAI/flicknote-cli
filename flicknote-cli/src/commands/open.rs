use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

use super::util::display_note_id;

#[derive(Args)]
pub(crate) struct OpenArgs {
    /// Note short ID. A full UUID is also accepted for pending-sync notes.
    id: String,
}

pub(crate) async fn run(db: &dyn NoteDb, config: &Config, args: &OpenArgs) -> Result<(), CliError> {
    let web_url = config.web_url.as_deref().ok_or_else(|| {
        CliError::Other(
            "webUrl not configured. Set it in ~/.config/flicknote/config.json or FLICKNOTE_WEB_URL."
                .into(),
        )
    })?;
    let full_id = db.resolve_note_id(&args.id).await?;
    let note = db.find_note(&full_id).await?;
    let display_id = display_note_id(&note);
    let url_id = note
        .short_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| full_id.clone());
    let url = format!("{}/notes/{}", web_url.trim_end_matches('/'), url_id);
    open::that(&url).map_err(CliError::Io)?;
    println!("Opened {} — {}", display_id, url);
    Ok(())
}
