use flicknote_auth::client::{AuthError, GoTrueClient};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Client, Method, Response};
use std::fmt;
use url::Url;

pub(crate) struct GatewayClient {
    auth: GoTrueClient,
    gateway_origin: Url,
    http: Client,
}

#[derive(Debug)]
pub(crate) enum GatewayRequestError {
    InvalidPath(CliError),
    NotAuthenticated,
    Session,
    Network,
    Authentication { status: reqwest::StatusCode },
    RateLimited { retry_after: Option<u64> },
    Upstream { status: reqwest::StatusCode },
}

impl GatewayRequestError {
    pub(crate) fn into_cli_error(self) -> CliError {
        match self {
            Self::InvalidPath(error) => error,
            Self::NotAuthenticated => CliError::NotAuthenticated,
            error => CliError::Other(error.to_string()),
        }
    }
}

impl fmt::Display for GatewayRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(error) => error.fmt(formatter),
            Self::NotAuthenticated => {
                write!(formatter, "Not authenticated — run `flicknote login`")
            }
            Self::Session => write!(
                formatter,
                "Unable to obtain a current FlickNote session; run `flicknote login` again."
            ),
            Self::Network => write!(formatter, "Gateway request failed"),
            Self::Authentication { status } => write!(
                formatter,
                "Gateway authentication failed (HTTP {status}); run `flicknote login` if needed."
            ),
            Self::RateLimited {
                retry_after: Some(seconds),
            } => write!(
                formatter,
                "Gateway rate limited the request (HTTP 429); retry after {seconds} seconds."
            ),
            Self::RateLimited { retry_after: None } => {
                write!(formatter, "Gateway rate limited the request (HTTP 429).")
            }
            Self::Upstream { status } => {
                write!(formatter, "Gateway request failed (HTTP {status}).")
            }
        }
    }
}

impl std::error::Error for GatewayRequestError {}

impl GatewayClient {
    pub(crate) fn new(config: &Config) -> Result<Self, CliError> {
        Ok(Self {
            auth: GoTrueClient::new(
                &config.supabase_url,
                &config.supabase_anon_key,
                &config.paths.session_file,
            ),
            gateway_origin: gateway_origin(&config.api_url)?,
            http: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| CliError::Http("Failed to configure Gateway client".into()))?,
        })
    }

    pub(crate) async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<Response, GatewayRequestError> {
        self.request_with_content_type(method, path, body, None)
            .await
    }

    pub(crate) async fn request_json(
        &self,
        method: Method,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<Response, GatewayRequestError> {
        let body = serde_json::to_vec(body).map_err(|_| GatewayRequestError::Network)?;
        self.request_json_bytes(method, path, &body).await
    }

    pub(crate) async fn request_json_bytes(
        &self,
        method: Method,
        path: &str,
        body: &[u8],
    ) -> Result<Response, GatewayRequestError> {
        self.request_with_content_type(method, path, Some(body), Some("application/json"))
            .await
    }

    async fn request_with_content_type(
        &self,
        method: Method,
        path: &str,
        body: Option<&[u8]>,
        content_type: Option<&str>,
    ) -> Result<Response, GatewayRequestError> {
        let url = gateway_path_url(&self.gateway_origin, path)
            .map_err(GatewayRequestError::InvalidPath)?;
        let session = self
            .auth
            .get_session()
            .await
            .map_err(|error| auth_error(&error))?;
        let mut request = self
            .http
            .request(method, url)
            .bearer_auth(session.access_token);
        if let Some(body) = body {
            request = request.body(body.to_vec());
        }
        if let Some(content_type) = content_type {
            request = request.header(CONTENT_TYPE, content_type);
        }
        let response = request
            .send()
            .await
            .map_err(|_| GatewayRequestError::Network)?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(gateway_response_error(&response))
        }
    }
}

fn auth_error(error: &AuthError) -> GatewayRequestError {
    match error {
        AuthError::NotAuthenticated => GatewayRequestError::NotAuthenticated,
        _ => GatewayRequestError::Session,
    }
}

fn gateway_response_error(response: &Response) -> GatewayRequestError {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(valid_retry_after);
    match status.as_u16() {
        401 | 403 => GatewayRequestError::Authentication { status },
        429 => GatewayRequestError::RateLimited { retry_after },
        _ => GatewayRequestError::Upstream { status },
    }
}

fn valid_retry_after(value: &str) -> Option<u64> {
    let seconds = value.trim().parse::<u64>().ok()?;
    Some(seconds)
}

#[cfg(test)]
fn gateway_url(api_url: &str, path: &str) -> Result<Url, CliError> {
    let origin = gateway_origin(api_url)?;
    gateway_path_url(&origin, path)
}

fn gateway_origin(api_url: &str) -> Result<Url, CliError> {
    let api_url = Url::parse(api_url).map_err(|_| {
        CliError::Other("Configured Gateway API URL is invalid; update apiUrl".into())
    })?;
    if !matches!(api_url.scheme(), "http" | "https")
        || api_url.host_str().is_none()
        || !api_url.username().is_empty()
        || api_url.password().is_some()
    {
        return Err(CliError::Other(
            "Configured Gateway API URL must be an HTTP(S) origin without credentials".into(),
        ));
    }

    let mut origin = api_url;
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);
    Ok(origin)
}

fn gateway_path_url(origin: &Url, path: &str) -> Result<Url, CliError> {
    if !path.starts_with('/') || path.starts_with("//") {
        return Err(CliError::Other(
            "Gateway path must start with a single `/` and cannot be a URL".into(),
        ));
    }
    let target = origin.join(path).map_err(|_| {
        CliError::Other("Gateway path must be a valid same-origin absolute path".into())
    })?;
    if target.origin() != origin.origin() {
        return Err(CliError::Other(
            "Gateway path must remain on the configured Gateway origin".into(),
        ));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flicknote_core::config::{Config, ConfigPaths};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn config(api_url: String, session_file: std::path::PathBuf) -> Config {
        Config {
            supabase_url: "https://auth.example.test".to_string(),
            supabase_anon_key: "anon-key".to_string(),
            powersync_url: String::new(),
            api_url,
            web_url: None,
            paths: ConfigPaths {
                config_dir: std::path::PathBuf::new(),
                data_dir: std::path::PathBuf::new(),
                config_file: std::path::PathBuf::new(),
                session_file,
                db_file: std::path::PathBuf::new(),
                log_file: std::path::PathBuf::new(),
            },
        }
    }

    fn write_session(path: &std::path::Path, token: &str, expires_at: Option<u64>) {
        let session = serde_json::json!({
            "access_token": token,
            "refresh_token": "test-refresh",
            "expires_at": expires_at,
            "user": { "id": "test-user", "email": null }
        });
        let wrapper = serde_json::json!({
            "sb-test-auth-token": serde_json::to_string(&session).unwrap()
        });
        std::fs::write(path, serde_json::to_vec(&wrapper).unwrap()).unwrap();
    }

    fn spawn_server(response: &'static str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let count = stream.read(&mut buffer).unwrap();
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&buffer[..count]).into_owned()
        });
        (format!("http://{address}"), handle)
    }

    fn spawn_server_sequence(responses: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            responses
                .into_iter()
                .map(|response| {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut buffer = [0_u8; 4096];
                    let count = stream.read(&mut buffer).unwrap();
                    stream.write_all(response.as_bytes()).unwrap();
                    String::from_utf8_lossy(&buffer[..count]).into_owned()
                })
                .collect()
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn gateway_url_uses_only_the_configured_origin_and_rejects_external_paths() {
        let api_url = "https://dev-gw.flicknote.app/api/v1";

        assert_eq!(
            gateway_url(api_url, "/web/v1/search?query=flicknote")
                .unwrap()
                .as_str(),
            "https://dev-gw.flicknote.app/web/v1/search?query=flicknote"
        );

        for path in [
            "https://example.com/web/v1/search",
            "//example.com/web/v1/search",
            "web/v1/search",
        ] {
            assert!(gateway_url(api_url, path).is_err(), "accepted {path}");
        }
    }

    #[tokio::test]
    async fn request_injects_the_session_bearer_token_and_never_follows_redirects() {
        let directory = tempfile::tempdir().unwrap();
        let session_file = directory.path().join("session.json");
        write_session(&session_file, "session-token", None);
        let (origin, server) = spawn_server(
            "HTTP/1.1 302 Found\r\nLocation: /other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let config = config(format!("{origin}/api/v1"), session_file);

        let error = GatewayClient::new(&config)
            .unwrap()
            .request(
                reqwest::Method::POST,
                "/web/v1/search",
                Some(br#"{"query":"rust"}"#),
            )
            .await
            .unwrap_err();

        let request = server.join().unwrap();
        assert!(request.starts_with("POST /web/v1/search HTTP/1.1\r\n"));
        assert!(request.contains("authorization: Bearer session-token\r\n"));
        assert!(request.contains("{\"query\":\"rust\"}"));
        assert!(format!("{error}").contains("302"));
    }

    #[tokio::test]
    async fn request_refreshes_an_expired_session_before_calling_the_gateway() {
        let directory = tempfile::tempdir().unwrap();
        let session_file = directory.path().join("session.json");
        write_session(&session_file, "expired-token", Some(0));
        let (origin, server) = spawn_server_sequence(vec![
            "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{\"access_token\":\"refreshed-token\",\"refresh_token\":\"refreshed-refresh\",\"expires_at\":4102444800,\"user\":{\"id\":\"test-user\",\"email\":null}}".to_string(),
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_string(),
        ]);
        let mut config = config(format!("{origin}/api/v1"), session_file.clone());
        config.supabase_url = origin;

        GatewayClient::new(&config)
            .unwrap()
            .request(reqwest::Method::GET, "/healthz", None)
            .await
            .unwrap();

        let requests = server.join().unwrap();
        assert!(
            requests[0].starts_with("POST /auth/v1/token?grant_type=refresh_token HTTP/1.1\r\n")
        );
        assert!(requests[1].starts_with("GET /healthz HTTP/1.1\r\n"));
        assert!(requests[1].contains("authorization: Bearer refreshed-token\r\n"));
        assert_eq!(
            flicknote_auth::session::load_session(&session_file)
                .unwrap()
                .access_token,
            "refreshed-token"
        );
    }

    #[tokio::test]
    async fn request_hides_refresh_failure_details() {
        let directory = tempfile::tempdir().unwrap();
        let session_file = directory.path().join("session.json");
        write_session(&session_file, "expired-token", Some(0));
        let (origin, server) = spawn_server(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 12\r\nConnection: close\r\n\r\nrefresh-token",
        );
        let mut config = config(format!("{origin}/api/v1"), session_file);
        config.supabase_url = origin;

        let error = GatewayClient::new(&config)
            .unwrap()
            .request(reqwest::Method::GET, "/healthz", None)
            .await
            .unwrap_err();

        server.join().unwrap();
        let message = error.to_string();
        assert!(message.contains("current FlickNote session"));
        assert!(!message.contains("refresh-token"));
        assert!(!message.contains("expired-token"));
    }
}
