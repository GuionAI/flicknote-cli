use clap::Args;
use flicknote_auth::client::{AuthError, GoTrueClient};
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use serde::Deserialize;

const SHARE_HELP: &str = include_str!("../help/share.md");

#[derive(Clone, Copy)]
enum ShareResource {
    Note,
    Project,
}

impl ShareResource {
    fn path_segment(self) -> &'static str {
        match self {
            Self::Note => "notes",
            Self::Project => "projects",
        }
    }

    fn missing_error_code(self) -> &'static str {
        match self {
            Self::Note => "SHARE_NOT_FOUND",
            Self::Project => "PROJECT_SHARE_NOT_FOUND",
        }
    }
}

#[derive(Deserialize)]
struct ShareResponse {
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiErrorResponse {
    error_code: Option<String>,
    message: Option<String>,
}

async fn response_error(response: reqwest::Response) -> CliError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<ApiErrorResponse>(&body)
        .ok()
        .and_then(|error| error.message)
        .unwrap_or(body);
    CliError::Http(format!("share API returned {status}: {message}"))
}

async fn share_url(response: reqwest::Response) -> Result<String, CliError> {
    if !response.status().is_success() {
        return Err(response_error(response).await);
    }
    let share = response
        .json::<ShareResponse>()
        .await
        .map_err(|error| CliError::Http(error.to_string()))?;
    Ok(share.url)
}

fn share_endpoint(api_url: &str, resource: ShareResource, id: &str) -> String {
    let versioned_base = api_url
        .trim_end_matches('/')
        .trim_end_matches("/api/v1")
        .trim_end_matches('/');
    format!(
        "{versioned_base}/api/v1/{}/{id}/share",
        resource.path_segment()
    )
}

async fn get_or_create_share(
    http: &reqwest::Client,
    api_url: &str,
    access_token: &str,
    resource: ShareResource,
    id: &str,
) -> Result<String, CliError> {
    let endpoint = share_endpoint(api_url, resource, id);
    let response = http
        .get(&endpoint)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| CliError::Http(error.to_string()))?;

    if response.status().is_success() {
        return share_url(response).await;
    }

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| CliError::Http(error.to_string()))?;
    let is_missing_share = status == reqwest::StatusCode::NOT_FOUND
        && serde_json::from_str::<ApiErrorResponse>(&body)
            .ok()
            .and_then(|error| error.error_code)
            .is_some_and(|code| code == resource.missing_error_code());
    if !is_missing_share {
        let message = serde_json::from_str::<ApiErrorResponse>(&body)
            .ok()
            .and_then(|error| error.message)
            .unwrap_or(body);
        return Err(CliError::Http(format!(
            "share API returned {status}: {message}"
        )));
    }

    let response = http
        .post(&endpoint)
        .bearer_auth(access_token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|error| CliError::Http(error.to_string()))?;
    share_url(response).await
}

async fn share(config: &Config, resource: ShareResource, id: &str) -> Result<String, CliError> {
    config.validate_api()?;
    let auth = GoTrueClient::new(
        &config.supabase_url,
        &config.supabase_anon_key,
        &config.paths.session_file,
    );
    let session = auth.get_session().await.map_err(|error| match error {
        AuthError::NotAuthenticated => CliError::NotAuthenticated,
        other => CliError::Auth {
            operation: "session refresh".into(),
            description: other.to_string(),
        },
    })?;
    get_or_create_share(
        &reqwest::Client::new(),
        &config.api_url,
        &session.access_token,
        resource,
        id,
    )
    .await
}

#[derive(Args)]
#[command(after_help = SHARE_HELP)]
pub(crate) struct ShareArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    pub(crate) id: String,
}

pub(crate) async fn run_note(
    db: &dyn NoteDb,
    config: &Config,
    args: &ShareArgs,
) -> Result<(), CliError> {
    let id = db.resolve_note_id(&args.id).await?;
    let url = share(config, ShareResource::Note, &id).await?;
    println!("{url}");
    Ok(())
}

pub(crate) async fn run_project(
    db: &dyn NoteDb,
    config: &Config,
    id: &str,
) -> Result<(), CliError> {
    let id = db.resolve_project_id(id).await?;
    let url = share(config, ShareResource::Project, &id).await?;
    println!("{url}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    fn spawn_server(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0_u8; 4096];
                let count = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..count]);
                requests.push(request.lines().next().unwrap_or_default().to_string());
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (format!("http://{address}"), handle)
    }

    #[tokio::test]
    async fn returns_existing_note_share_without_replacing_it() {
        let (api_origin, server) = spawn_server(vec![(
            "200 OK",
            r#"{"token":"existing","url":"https://flicknote.app/s/existing"}"#,
        )]);
        let api_url = format!("{api_origin}/api/v1");

        let url = get_or_create_share(
            &reqwest::Client::new(),
            &api_url,
            "access-token",
            ShareResource::Note,
            "550e8400-e29b-41d4-a716-446655440000",
        )
        .await
        .unwrap();

        assert_eq!(url, "https://flicknote.app/s/existing");
        assert_eq!(
            server.join().unwrap(),
            ["GET /api/v1/notes/550e8400-e29b-41d4-a716-446655440000/share HTTP/1.1"]
        );
    }

    #[tokio::test]
    async fn creates_project_share_when_none_exists() {
        let (api_url, server) = spawn_server(vec![
            (
                "404 Not Found",
                r#"{"_tag":"NotFoundError","message":"No project share link exists for this project","errorCode":"PROJECT_SHARE_NOT_FOUND"}"#,
            ),
            (
                "200 OK",
                r#"{"token":"new-token","url":"https://flicknote.app/p/new-token"}"#,
            ),
        ]);

        let url = get_or_create_share(
            &reqwest::Client::new(),
            &api_url,
            "access-token",
            ShareResource::Project,
            "550e8400-e29b-41d4-a716-446655440000",
        )
        .await
        .unwrap();

        assert_eq!(url, "https://flicknote.app/p/new-token");
        assert_eq!(
            server.join().unwrap(),
            [
                "GET /api/v1/projects/550e8400-e29b-41d4-a716-446655440000/share HTTP/1.1",
                "POST /api/v1/projects/550e8400-e29b-41d4-a716-446655440000/share HTTP/1.1",
            ]
        );
    }
}
