use crate::client::{AuthError, AuthSession};
use std::path::Path;

/// Load session from the Supabase-compatible session.json
pub fn load_session(path: &Path) -> Result<AuthSession, AuthError> {
    let raw = std::fs::read_to_string(path).map_err(|_| AuthError::NotAuthenticated)?;
    let data: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| AuthError::NotAuthenticated)?;
    let obj = data.as_object().ok_or(AuthError::NotAuthenticated)?;
    let token_key = obj
        .keys()
        .find(|k| k.ends_with("-auth-token"))
        .ok_or(AuthError::NotAuthenticated)?
        .clone();
    let token_str = obj[&token_key]
        .as_str()
        .ok_or(AuthError::NotAuthenticated)?;
    serde_json::from_str(token_str).map_err(|_| AuthError::NotAuthenticated)
}

/// Save session in Supabase-compatible format
pub fn save_session(path: &Path, session: &AuthSession) -> Result<(), AuthError> {
    let mut data: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();

    let key = data
        .keys()
        .find(|k| k.ends_with("-auth-token"))
        .cloned()
        .unwrap_or_else(|| "sb-dev-auth-token".to_string());

    // Supabase JS SDK stores the session as a stringified JSON value
    let session_str = serde_json::to_string(session)?;
    data.insert(key, serde_json::Value::String(session_str));

    let json = serde_json::to_string_pretty(&data)?;

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(json.as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, &json)?;

    Ok(())
}
