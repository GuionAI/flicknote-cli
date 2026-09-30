//! Explicit remote MCP entrypoint with per-request verifier and PostgreSQL identity.
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, Response, StatusCode, header};
use axum::routing::any;
use axum::{Json, Router, response::IntoResponse};
use deadpool_postgres::{Manager, Pool};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_postgres::NoTls;
use tower_service::Service;
use uuid::Uuid;

use crate::app::Application;
use crate::mcp::FlickNoteMcp;
use crate::pg::{PgNoteCreator, PgRequestDb, PgSearch};

const FULL_SCOPE: &str = "flicknote:full";

#[derive(Clone)]
pub struct PrivateMcpConfig {
    pub database_url: String,
    pub listen: SocketAddr,
    pub resource: String,
    pub issuer: String,
    pub verifier: String,
    pub allowed_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
}

impl PrivateMcpConfig {
    pub fn validate(&self) -> Result<(), String> {
        let resource = validate_url(&self.resource, true)?;
        validate_url(&self.issuer, false)?;
        validate_url(&self.verifier, false)?;
        if self.allowed_hosts.is_empty() || self.allowed_origins.is_empty() {
            return Err("allowed hosts and origins must be explicitly configured".into());
        }
        if self
            .allowed_hosts
            .iter()
            .any(|value| value == "*" || value.is_empty())
            || self
                .allowed_origins
                .iter()
                .any(|value| value == "*" || value.is_empty())
        {
            return Err("wildcard host/origin policy is forbidden".into());
        }
        for origin in &self.allowed_origins {
            let url = validate_url(origin, false)?;
            if url.path() != "/" || url.query().is_some() {
                return Err("allowed origins must contain only scheme and authority".into());
            }
        }
        if resource.path() != "/mcp" || resource.query().is_some() {
            return Err("resource must be the canonical /mcp URL".into());
        }
        if self.database_url.trim().is_empty() {
            return Err("database URL is required".into());
        }
        Ok(())
    }
    fn metadata_url(&self) -> String {
        let url = reqwest::Url::parse(&self.resource).expect("validated resource");
        format!(
            "{}://{}/.well-known/oauth-protected-resource/mcp",
            url.scheme(),
            url.authority()
        )
    }
}
fn validate_url(value: &str, resource: bool) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "invalid remote URL".to_string())?;
    if url.username() != "" || url.password().is_some() || url.fragment().is_some() {
        return Err("remote URLs must not contain credentials or fragments".into());
    }
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !(loopback && url.scheme() == "http") {
        return Err("remote URLs require HTTPS except explicit loopback development URLs".into());
    }
    if resource && url.path() != "/mcp" {
        return Err("resource URL must end at /mcp".into());
    }
    Ok(url)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Verification {
    ok: bool,
    user_id: Option<String>,
    scope: Option<Vec<String>>,
    resource: Option<String>,
}
#[derive(Debug, PartialEq, Eq)]
enum VerifyError {
    Invalid,
    Unavailable,
}

#[derive(Clone)]
struct PrivateState {
    config: PrivateMcpConfig,
    verifier_client: reqwest::Client,
    pool: Pool,
}
impl PrivateState {
    async fn verify(&self, token: &str) -> Result<Uuid, VerifyError> {
        let response = self
            .verifier_client
            .post(&self.config.verifier)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| VerifyError::Unavailable)?;
        match response.status() {
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                return Err(VerifyError::Invalid);
            }
            status if status.is_server_error() => return Err(VerifyError::Unavailable),
            reqwest::StatusCode::OK => {}
            _ => return Err(VerifyError::Unavailable),
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| VerifyError::Unavailable)?;
        let result: Verification =
            serde_json::from_slice(&bytes).map_err(|_| VerifyError::Invalid)?;
        if !result.ok {
            return Err(VerifyError::Invalid);
        }
        if result.resource.as_deref() != Some(&self.config.resource)
            || !result
                .scope
                .as_ref()
                .is_some_and(|scope| scope.iter().any(|value| value == FULL_SCOPE))
        {
            return Err(VerifyError::Invalid);
        }
        result
            .user_id
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(VerifyError::Invalid)
    }
    fn challenge(&self) -> Response<Body> {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::UNAUTHORIZED;
        let value = format!(
            "Bearer resource_metadata=\"{}\", scope=\"{}\"",
            self.config.metadata_url(),
            FULL_SCOPE
        );
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            value.parse().expect("valid challenge"),
        );
        response
    }
    fn headers_allowed(&self, headers: &HeaderMap) -> bool {
        if headers.contains_key("forwarded")
            || headers
                .keys()
                .any(|name| name.as_str().starts_with("x-forwarded-"))
        {
            return false;
        }
        let Some(host) = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
        else {
            return false;
        };
        if !self.config.allowed_hosts.iter().any(|value| value == host) {
            return false;
        }
        if let Some(origin) = headers.get(header::ORIGIN) {
            let Some(origin) = origin.to_str().ok() else {
                return false;
            };
            if !self
                .config
                .allowed_origins
                .iter()
                .any(|value| value == origin)
            {
                return false;
            }
        }
        true
    }
}

#[derive(Serialize)]
struct ProtectedResource<'a> {
    resource: &'a str,
    authorization_servers: [&'a str; 1],
    scopes_supported: [&'a str; 1],
}
async fn metadata(State(state): State<PrivateState>, request: Request<Body>) -> Response<Body> {
    if !state.headers_allowed(request.headers()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    Json(ProtectedResource {
        resource: &state.config.resource,
        authorization_servers: [&state.config.issuer],
        scopes_supported: [FULL_SCOPE],
    })
    .into_response()
}
async fn mcp(State(state): State<PrivateState>, request: Request<Body>) -> Response<Body> {
    if !state.headers_allowed(request.headers()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(token) = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, token)| {
            scheme.eq_ignore_ascii_case("Bearer") && !token.is_empty() && !token.contains(' ')
        })
        .map(|(_, token)| token)
    else {
        return state.challenge();
    };
    let user = match state.verify(token).await {
        Ok(user) => user,
        Err(VerifyError::Invalid) => return state.challenge(),
        Err(VerifyError::Unavailable) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let db = match PgRequestDb::begin(&state.pool, user).await {
        Ok(db) => db,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let app = Arc::new(
        Application::new_private(db.clone(), Arc::new(PgNoteCreator(db.clone())))
            .with_search(PgSearch(db.clone())),
    );
    let db_for_factory = db.clone();
    let mut config = StreamableHttpServerConfig::default();
    config.legacy_session_mode = false;
    config.json_response = true;
    config.allowed_hosts = state.config.allowed_hosts.clone();
    config.allowed_origins = state.config.allowed_origins.clone();
    let mut service: StreamableHttpService<FlickNoteMcp, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(FlickNoteMcp::new_remote(
                    app.clone(),
                    db_for_factory.clone(),
                ))
            },
            Default::default(),
            config,
        );
    let response = service.call(request).await.expect("infallible MCP service");
    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(Body::new(body), 8 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            drop(db.finish(false).await);
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    if db.finish(!db.failed()).await.is_err() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    Response::from_parts(parts, Body::from(bytes))
}

pub async fn serve(config: PrivateMcpConfig) -> Result<(), String> {
    config.validate()?;
    let pg_config = tokio_postgres::Config::from_str(&config.database_url)
        .map_err(|_| "invalid database URL".to_string())?;
    let pool = Pool::builder(Manager::new(pg_config, NoTls))
        .max_size(16)
        .build()
        .map_err(|_| "PostgreSQL pool configuration failed".to_string())?;
    let verifier_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "verifier client initialization failed".to_string())?;
    let listen = config.listen;
    let state = PrivateState {
        config,
        verifier_client,
        pool,
    };
    let router = Router::new()
        .route("/.well-known/oauth-protected-resource/mcp", any(metadata))
        .route("/mcp", any(mcp))
        .with_state(state);
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|error| format!("listen failed: {error}"))?;
    axum::serve(listener, router)
        .await
        .map_err(|error| format!("server stopped: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;
    use tokio_postgres::NoTls;

    async fn fixture() -> (PrivateState, tokio::task::JoinHandle<()>) {
        async fn verify(headers: HeaderMap) -> Response<Body> {
            let token = headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("");
            let payload = match token {
                "Bearer full" => {
                    serde_json::json!({"ok":true,"userId":"11111111-1111-4111-8111-111111111111","scope":["flicknote:full"],"resource":"http://localhost:3000/mcp","extra":42})
                }
                "Bearer partial" => {
                    serde_json::json!({"ok":true,"userId":"11111111-1111-4111-8111-111111111111","scope":["note:read"],"resource":"http://localhost:3000/mcp"})
                }
                "Bearer resource" => {
                    serde_json::json!({"ok":true,"userId":"11111111-1111-4111-8111-111111111111","scope":["flicknote:full"],"resource":"http://localhost:3001/mcp"})
                }
                "Bearer malformed" => {
                    serde_json::json!({"ok":true,"userId":"not-a-uuid","scope":["flicknote:full"],"resource":"http://localhost:3000/mcp"})
                }
                "Bearer expired" => return StatusCode::UNAUTHORIZED.into_response(),
                "Bearer outage" => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
                _ => serde_json::json!({"ok":false}),
            };
            Json(payload).into_response()
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/verify", post(verify)))
                .await
                .unwrap();
        });
        let mut pg = tokio_postgres::Config::new();
        pg.host("127.0.0.1").port(1).user("unused").dbname("unused");
        let pool = Pool::builder(Manager::new(pg, NoTls))
            .max_size(1)
            .build()
            .unwrap();
        let config = PrivateMcpConfig {
            database_url: "postgresql://unused@localhost/unused".into(),
            listen: "127.0.0.1:0".parse().unwrap(),
            resource: "http://localhost:3000/mcp".into(),
            issuer: "http://localhost:3000/oauth".into(),
            verifier: format!("http://127.0.0.1:{port}/verify"),
            allowed_hosts: vec!["localhost:3000".into()],
            allowed_origins: vec!["http://localhost:3000".into()],
        };
        (
            PrivateState {
                config,
                verifier_client: reqwest::Client::new(),
                pool,
            },
            task,
        )
    }
    fn request(token: Option<&str>, host: &str, origin: Option<&str>) -> Request<Body> {
        let mut request = Request::builder().uri("/mcp").header(header::HOST, host);
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        request.body(Body::empty()).unwrap()
    }
    #[tokio::test]
    async fn verifier_requires_full_resource_bound_grant() {
        let (state, task) = fixture().await;
        assert_eq!(
            state.verify("full").await.unwrap().to_string(),
            "11111111-1111-4111-8111-111111111111"
        );
        for token in ["partial", "resource", "malformed", "expired"] {
            assert_eq!(state.verify(token).await, Err(VerifyError::Invalid));
        }
        assert_eq!(state.verify("outage").await, Err(VerifyError::Unavailable));
        task.abort();
    }
    #[tokio::test]
    async fn verifier_body_transport_failure_is_unavailable_but_complete_bad_json_is_invalid() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut state, fixture_task) = fixture().await;
        fixture_task.abort();
        state.verifier_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        for (length, stall, expected) in [
            (20, true, StatusCode::SERVICE_UNAVAILABLE),
            (20, false, StatusCode::SERVICE_UNAVAILABLE),
            (1, false, StatusCode::UNAUTHORIZED),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            state.config.verifier = format!("http://{}/verify", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 2048];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{{").as_bytes()).await.unwrap();
                if stall {
                    std::future::pending::<()>().await;
                }
            });
            let response = mcp(
                State(state.clone()),
                request(Some("full"), "localhost:3000", None),
            )
            .await;
            server.abort();
            assert_eq!(
                response.status(),
                expected,
                "length={length}, stall={stall}"
            );
        }
    }

    #[tokio::test]
    async fn metadata_challenge_and_host_origin_policy() {
        let (state, task) = fixture().await;
        let missing = mcp(State(state.clone()), request(None, "localhost:3000", None)).await;
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        assert!(
            missing
                .headers()
                .get(header::WWW_AUTHENTICATE)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("/.well-known/oauth-protected-resource/mcp")
        );
        let partial = mcp(
            State(state.clone()),
            request(Some("partial"), "localhost:3000", None),
        )
        .await;
        assert_eq!(partial.status(), StatusCode::UNAUTHORIZED);
        let outage = mcp(
            State(state.clone()),
            request(Some("outage"), "localhost:3000", None),
        )
        .await;
        assert_eq!(outage.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bad_host = mcp(
            State(state.clone()),
            request(Some("full"), "evil.test", None),
        )
        .await;
        assert_eq!(bad_host.status(), StatusCode::FORBIDDEN);
        let bad_origin = mcp(
            State(state.clone()),
            request(Some("full"), "localhost:3000", Some("http://evil.test")),
        )
        .await;
        assert_eq!(bad_origin.status(), StatusCode::FORBIDDEN);
        let metadata = metadata(State(state), request(None, "localhost:3000", None)).await;
        assert_eq!(metadata.status(), StatusCode::OK);
        task.abort();
    }
    #[test]
    fn config_rejects_public_plaintext_and_wildcards() {
        let mut config = PrivateMcpConfig {
            database_url: "postgresql://test@localhost/test".into(),
            listen: "127.0.0.1:0".parse().unwrap(),
            resource: "http://example.test/mcp".into(),
            issuer: "https://example.test/oauth".into(),
            verifier: "https://example.test/verify".into(),
            allowed_hosts: vec!["example.test".into()],
            allowed_origins: vec!["https://example.test".into()],
        };
        assert!(config.validate().is_err());
        config.resource = "https://example.test/mcp".into();
        assert!(config.validate().is_ok());
        config.allowed_hosts = vec!["*".into()];
        assert!(config.validate().is_err());
    }
}
