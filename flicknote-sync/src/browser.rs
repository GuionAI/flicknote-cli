use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::BrowserOpener;

pub struct SystemBrowserOpener;

impl BrowserOpener for SystemBrowserOpener {
    fn open(&self, url: &str) -> Result<(), ServiceError> {
        open::that(url).map_err(ServiceError::Io)
    }
}
