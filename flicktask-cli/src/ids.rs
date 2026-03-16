use anyhow::{Context, Result, bail};
use taskchampion::{Replica, Uuid};

/// Format a UUID as an 8-char lowercase hex prefix.
pub fn short_id(uuid: &Uuid) -> String {
    uuid.to_string().replace('-', "")[..8].to_string()
}

#[cfg(feature = "powersync")]
use taskchampion::PowerSyncStorage;

/// Resolve an 8-char hex prefix or full UUID string to a task UUID.
/// Normalizes input to lowercase before matching.
#[cfg(feature = "powersync")]
pub async fn resolve_id(replica: &mut Replica<PowerSyncStorage>, input: &str) -> Result<Uuid> {
    let normalized = input.to_lowercase();

    // Try full UUID parse first
    if let Ok(uuid) = Uuid::parse_str(&normalized) {
        return Ok(uuid);
    }

    // Prefix match — must be exactly 8 hex chars
    if normalized.len() != 8 || !normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("Invalid task ID: {input:?} — use an 8-char hex prefix or full UUID");
    }

    let all_uuids = replica
        .all_task_uuids()
        .await
        .context("Failed to list task UUIDs")?;

    let matches: Vec<Uuid> = all_uuids
        .into_iter()
        .filter(|uuid| uuid.to_string().replace('-', "").starts_with(&normalized))
        .collect();

    match matches.len() {
        0 => bail!("No task found with ID prefix: {input}"),
        1 => Ok(matches[0]),
        _ => {
            let ids: Vec<String> = matches
                .iter()
                .map(|u| u.to_string()[..8].to_string())
                .collect();
            bail!(
                "Ambiguous ID {input:?} — matches: {}. Use more characters.",
                ids.join(", ")
            )
        }
    }
}
