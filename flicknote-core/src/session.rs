use crate::config::Config;
use crate::error::CliError;

/// Read the cached user ID from session.json using the auth crate's session parser.
pub fn get_user_id(config: &Config) -> Result<String, CliError> {
    let session = flicknote_auth::session::load_session(&config.paths.session_file)
        .map_err(|_| CliError::NotAuthenticated)?;
    Ok(session.user.id)
}
