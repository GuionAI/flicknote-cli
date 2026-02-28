use crate::pkce;
use crate::session::{load_session, save_session};
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct GoTrueClient {
    http: Client,
    gotrue_url: String,
    anon_key: String,
    session_file: std::path::PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthSession {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: Option<u64>,
    pub user: AuthUser,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub email: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("API error: {0}")]
    Api(String),
    #[error("OAuth login timed out after 60 seconds")]
    Timeout,
    #[error("Not authenticated — run `flicknote login`")]
    NotAuthenticated,
}

impl GoTrueClient {
    pub fn new(
        supabase_url: &str,
        anon_key: &str,
        session_file: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            http: Client::new(),
            gotrue_url: format!("{supabase_url}/auth/v1"),
            anon_key: anon_key.to_string(),
            session_file: session_file.into(),
        }
    }

    /// POST /otp — send a one-time password to email
    pub async fn sign_in_with_otp(&self, email: &str) -> Result<(), AuthError> {
        let pkce = pkce::generate();
        save_pkce_verifier(&self.session_file, &pkce.verifier)?;

        let resp = self
            .http
            .post(format!("{}/otp", self.gotrue_url))
            .header("apikey", &self.anon_key)
            .json(&serde_json::json!({
                "email": email,
                "code_challenge": pkce.challenge,
                "code_challenge_method": "s256",
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AuthError::Api(format!("OTP failed: {body}")));
        }
        Ok(())
    }

    /// POST /verify — verify OTP code, completing the PKCE handshake
    pub async fn verify_otp(&self, email: &str, code: &str) -> Result<AuthSession, AuthError> {
        let verifier = load_pkce_verifier(&self.session_file)?;

        let resp = self
            .http
            .post(format!("{}/verify", self.gotrue_url))
            .header("apikey", &self.anon_key)
            .json(&serde_json::json!({
                "email": email,
                "token": code,
                "type": "email",
                "code_verifier": verifier,
            }))
            .send()
            .await?;

        cleanup_pkce_verifier(&self.session_file);

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AuthError::Api(format!("Verify failed: {body}")));
        }

        let session: AuthSession = resp.json().await?;
        self.persist_session(&session)?;
        Ok(session)
    }

    /// OAuth PKCE flow: build URL, start callback server, open browser, exchange code
    pub async fn sign_in_with_oauth(&self, provider: &str) -> Result<AuthSession, AuthError> {
        let pkce = pkce::generate();

        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let redirect_url = format!("http://127.0.0.1:{port}/callback");

        let server_handle = tokio::spawn(crate::oauth::wait_for_callback(listener, tx));

        let oauth_url = format!(
            "{}/authorize?provider={provider}&redirect_to={}&code_challenge={}&code_challenge_method=s256&skip_http_redirect=true",
            self.gotrue_url,
            urlencoding::encode(&redirect_url),
            pkce.challenge,
        );

        #[cfg(target_os = "macos")]
        std::process::Command::new("open").arg(&oauth_url).spawn()?;
        #[cfg(target_os = "linux")]
        std::process::Command::new("xdg-open")
            .arg(&oauth_url)
            .spawn()?;

        println!("Opened browser for {provider} login...");

        let code = tokio::time::timeout(std::time::Duration::from_secs(60), rx)
            .await
            .map_err(|_| AuthError::Timeout)?
            .map_err(|_| AuthError::Api("Callback channel closed".into()))?;

        server_handle.abort();

        self.exchange_code_for_session(&code, &pkce.verifier).await
    }

    /// POST /token?grant_type=pkce — exchange auth code for session
    async fn exchange_code_for_session(
        &self,
        code: &str,
        verifier: &str,
    ) -> Result<AuthSession, AuthError> {
        let resp = self
            .http
            .post(format!("{}/token?grant_type=pkce", self.gotrue_url))
            .header("apikey", &self.anon_key)
            .json(&serde_json::json!({
                "auth_code": code,
                "code_verifier": verifier,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AuthError::Api(format!("Code exchange failed: {body}")));
        }

        let session: AuthSession = resp.json().await?;
        self.persist_session(&session)?;
        Ok(session)
    }

    /// Read session from file, refresh if expired
    pub async fn get_session(&self) -> Result<AuthSession, AuthError> {
        let stored = load_session(&self.session_file)?;

        if let Some(expires_at) = stored.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if now + 60 >= expires_at {
                return self.refresh_token(&stored.refresh_token).await;
            }
        }

        Ok(stored)
    }

    /// POST /token?grant_type=refresh_token
    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthSession, AuthError> {
        let resp = self
            .http
            .post(format!(
                "{}/token?grant_type=refresh_token",
                self.gotrue_url
            ))
            .header("apikey", &self.anon_key)
            .json(&serde_json::json!({ "refresh_token": refresh_token }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(AuthError::Api("Token refresh failed".into()));
        }

        let session: AuthSession = resp.json().await?;
        self.persist_session(&session)?;
        Ok(session)
    }

    fn persist_session(&self, session: &AuthSession) -> Result<(), AuthError> {
        save_session(&self.session_file, session)
    }
}

fn save_pkce_verifier(session_file: &std::path::Path, verifier: &str) -> Result<(), AuthError> {
    let verifier_file = session_file.with_extension("pkce");
    std::fs::write(&verifier_file, verifier)?;
    Ok(())
}

fn load_pkce_verifier(session_file: &std::path::Path) -> Result<String, AuthError> {
    let verifier_file = session_file.with_extension("pkce");
    std::fs::read_to_string(&verifier_file)
        .map_err(|_| AuthError::Api("PKCE verifier not found — run login again".into()))
}

fn cleanup_pkce_verifier(session_file: &std::path::Path) {
    let verifier_file = session_file.with_extension("pkce");
    let _ = std::fs::remove_file(verifier_file);
}
