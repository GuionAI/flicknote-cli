use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::session;
use rusqlite::params;

#[derive(Args)]
pub struct ClipArgs {
    /// URL to clip
    url: String,
    /// Note title
    #[arg(long)]
    title: Option<String>,
    /// URL scheme input (flicknote://clip?url=...&title=...)
    #[arg(long)]
    url_scheme: Option<String>,
}

pub fn run(db: &Database, config: &Config, args: &ClipArgs) -> Result<(), CliError> {
    let user_id = session::get_user_id(config)?;

    let (url, title) = if let Some(ref scheme) = args.url_scheme {
        parse_url_scheme(scheme).unwrap_or((args.url.clone(), args.title.clone()))
    } else {
        (args.url.clone(), args.title.clone())
    };

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let metadata = serde_json::json!({ "link": { "url": url } }).to_string();

    db.write(|conn| {
        conn.execute(
            "INSERT INTO notes (id, user_id, type, status, title, metadata, created_at, updated_at)
             VALUES (?, ?, 'link', 'source_queued', ?, ?, ?, ?)",
            params![id, user_id, title, metadata, now, now],
        )?;
        Ok(())
    })?;

    println!(
        "{}",
        serde_json::json!({ "id": id, "status": "source_queued" })
    );
    Ok(())
}

fn parse_url_scheme(scheme: &str) -> Option<(String, Option<String>)> {
    let parsed = url::Url::parse(scheme).ok()?;
    let url = parsed
        .query_pairs()
        .find(|(k, _)| k == "url")?
        .1
        .to_string();
    let title = parsed
        .query_pairs()
        .find(|(k, _)| k == "title")
        .map(|(_, v)| v.to_string());
    Some((url, title))
}
