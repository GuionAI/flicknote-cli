use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::note::NoteService;
use flicknote_core::services::ports::BrowserOpener;

#[derive(Args)]
pub(crate) struct OpenArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted.
    id: String,
}

pub(crate) struct SystemBrowserOpener;

impl BrowserOpener for SystemBrowserOpener {
    fn open(&self, url: &str) -> Result<(), ServiceError> {
        open::that(url).map_err(ServiceError::Io)
    }
}

pub(crate) async fn run(db: &dyn NoteDb, config: &Config, args: &OpenArgs) -> Result<(), CliError> {
    let web_url = config.web_url.as_deref().ok_or_else(|| {
        CliError::Other(
            "webUrl not configured. Set it in ~/.config/flicknote/config.json or FLICKNOTE_WEB_URL."
                .into(),
        )
    })?;
    let result = NoteService::new(db)
        .open(&SystemBrowserOpener, web_url, &args.id)
        .await?;
    println!("Opened {}", result.url);
    Ok(())
}
