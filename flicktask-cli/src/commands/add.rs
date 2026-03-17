use anyhow::{Context, Result};
use clap::Args;
use taskchampion::chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Utc, Weekday};
use taskchampion::{Operations, PowerSyncStorage, Replica, Status, Tag, Uuid};

use crate::ids::{resolve_id, short_id};

#[derive(Args)]
pub struct AddArgs {
    /// Task description
    pub description: String,

    /// Parent task ID (8-char hex or full UUID)
    #[arg(long)]
    pub parent: Option<String>,

    /// Due date — supports YYYY-MM-DD or relative (today, tomorrow, 2days, eod, etc.)
    #[arg(long)]
    pub due: Option<String>,

    /// Scheduled date — supports YYYY-MM-DD or relative (today, tomorrow, 2days, eod, etc.)
    #[arg(long)]
    pub scheduled: Option<String>,

    /// Priority (H, M, or L)
    #[arg(long)]
    pub priority: Option<String>,

    /// Tag to add (repeatable)
    #[arg(long = "tag", short = 't')]
    pub tags: Vec<String>,

    /// Project name
    #[arg(long)]
    pub project: Option<String>,

    /// Set a UDA value (key=value, repeatable)
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: AddArgs) -> Result<()> {
    let mut ops = Operations::new();

    let uuid = Uuid::new_v4();
    let mut task = replica
        .create_task(uuid, &mut ops)
        .await
        .context("Failed to create task")?;

    task.set_description(args.description, &mut ops)?;
    task.set_status(Status::Pending, &mut ops)?;

    let now = taskchampion::chrono::Utc::now();
    task.set_value("entry", Some(now.timestamp().to_string()), &mut ops)?;

    if let Some(parent_id) = args.parent {
        let parent_uuid = resolve_id(replica, &parent_id).await?;
        task.set_value("parent", Some(parent_uuid.to_string()), &mut ops)?;
    }

    if let Some(due_str) = args.due {
        let due = parse_date(&due_str)?;
        task.set_due(Some(due), &mut ops)?;
    }

    if let Some(sched_str) = args.scheduled {
        let sched = parse_date(&sched_str)?;
        task.set_value("scheduled", Some(sched.timestamp().to_string()), &mut ops)?;
    }

    if let Some(priority) = args.priority {
        task.set_priority(priority, &mut ops)?;
    }

    for tag_str in args.tags {
        let tag: Tag = tag_str
            .parse()
            .with_context(|| format!("Invalid tag: {tag_str:?}"))?;
        task.add_tag(&tag, &mut ops)?;
    }

    if let Some(project) = args.project {
        task.set_value("project", Some(project), &mut ops)?;
    }

    for kv in args.set {
        let (key, value) = parse_kv(&kv)?;
        task.set_value(key, Some(value.to_string()), &mut ops)?;
    }

    // Run on-add hooks — may enrich task (e.g. add branch, project_path)
    let task_json = crate::tw_json::task_to_tw_json(&uuid.to_string(), &task);
    let final_json = crate::hooks::run_on_add(&task_json)?;
    super::apply_hook_fields(&final_json, &task_json, &mut task, &mut ops)?;

    replica
        .commit_operations(ops)
        .await
        .context("Failed to commit task")?;

    println!("{}", short_id(&uuid));

    Ok(())
}

pub fn parse_date(s: &str) -> Result<taskchampion::chrono::DateTime<Utc>> {
    // Try absolute YYYY-MM-DD first (interpreted as local date, converted to UTC)
    if let Ok(naive) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(naive
            .and_hms_opt(0, 0, 0)
            .expect("midnight valid")
            .and_local_timezone(Local)
            .single()
            .context("ambiguous local time")?
            .with_timezone(&Utc));
    }

    let now = Utc::now();
    let local_now = Local::now();
    let today_local = local_now.date_naive();
    let local_midnight = |date: NaiveDate| -> Result<taskchampion::chrono::DateTime<Utc>> {
        Ok(date
            .and_hms_opt(0, 0, 0)
            .expect("midnight valid")
            .and_local_timezone(Local)
            .single()
            .context("ambiguous local time")?
            .with_timezone(&Utc))
    };

    match s.to_lowercase().as_str() {
        "now" => return Ok(now),
        "today" => return local_midnight(today_local),
        "tomorrow" => return local_midnight(today_local + Duration::days(1)),
        "yesterday" => return local_midnight(today_local - Duration::days(1)),
        "eod" => {
            // End of day = local 23:59:59
            let eod = today_local
                .and_hms_opt(23, 59, 59)
                .expect("valid time")
                .and_local_timezone(Local)
                .single()
                .context("ambiguous local time")?
                .with_timezone(&Utc);
            return Ok(eod);
        }
        "eow" => {
            // End of week = next Saturday local midnight
            let days_until_sat = (Weekday::Sat.num_days_from_monday() as i64
                - local_now.weekday().num_days_from_monday() as i64
                + 7)
                % 7;
            let days = if days_until_sat == 0 {
                7
            } else {
                days_until_sat
            };
            return local_midnight(today_local + Duration::days(days));
        }
        "eom" => {
            // Last day of current month
            let (y, m) = if local_now.month() == 12 {
                (local_now.year() + 1, 1)
            } else {
                (local_now.year(), local_now.month() + 1)
            };
            let first_next = NaiveDate::from_ymd_opt(y, m, 1).expect("valid date");
            let last_day = first_next.pred_opt().expect("valid date");
            return local_midnight(last_day);
        }
        "eoy" => {
            let dec31 = NaiveDate::from_ymd_opt(local_now.year(), 12, 31).expect("valid date");
            return local_midnight(dec31);
        }
        "sow" => {
            // Start of next week (Monday)
            let days_until_mon = (Weekday::Mon.num_days_from_monday() as i64
                - local_now.weekday().num_days_from_monday() as i64
                + 7)
                % 7;
            let days = if days_until_mon == 0 {
                7
            } else {
                days_until_mon
            };
            return local_midnight(today_local + Duration::days(days));
        }
        "som" => {
            let (y, m) = if local_now.month() == 12 {
                (local_now.year() + 1, 1)
            } else {
                (local_now.year(), local_now.month() + 1)
            };
            let first = NaiveDate::from_ymd_opt(y, m, 1).expect("valid date");
            return local_midnight(first);
        }
        "soy" => {
            let jan1 = NaiveDate::from_ymd_opt(local_now.year() + 1, 1, 1).expect("valid date");
            return local_midnight(jan1);
        }
        "later" | "someday" => {
            let far = NaiveDate::from_ymd_opt(9999, 12, 30).expect("valid date");
            return Ok(Utc.from_utc_datetime(&far.and_hms_opt(0, 0, 0).expect("midnight valid")));
        }
        _ => {}
    }

    // Try weekday names (next occurring)
    let lower = s.to_lowercase();
    if let Some(target_wd) = parse_weekday(&lower) {
        let current_wd = local_now.weekday();
        let days_ahead = (target_wd.num_days_from_monday() as i64
            - current_wd.num_days_from_monday() as i64
            + 7)
            % 7;
        let days = if days_ahead == 0 { 7 } else { days_ahead };
        return local_midnight(today_local + Duration::days(days));
    }

    // Try relative durations: Ndays, Nday, Nda, Nhrs, Nh, Nwks, Nwk, Nmo
    // Day/week/month units resolve to local midnight; hours use wall-clock time.
    if let Some(dt) = parse_relative_duration(&lower, now, today_local)? {
        return Ok(dt);
    }

    anyhow::bail!(
        "Invalid date {s:?} — expected YYYY-MM-DD or relative date \
         (today, tomorrow, 2days, eod, eow, mon, etc.)"
    );
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    match s {
        "mon" | "monday" => Some(Weekday::Mon),
        "tue" | "tuesday" => Some(Weekday::Tue),
        "wed" | "wednesday" => Some(Weekday::Wed),
        "thu" | "thursday" => Some(Weekday::Thu),
        "fri" | "friday" => Some(Weekday::Fri),
        "sat" | "saturday" => Some(Weekday::Sat),
        "sun" | "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

fn parse_relative_duration(
    s: &str,
    now: taskchampion::chrono::DateTime<Utc>,
    today_local: taskchampion::chrono::NaiveDate,
) -> Result<Option<taskchampion::chrono::DateTime<Utc>>> {
    // Extract leading digits
    let Some(num_end) = s.find(|c: char| !c.is_ascii_digit()) else {
        return Ok(None);
    };
    let Ok(n) = s[..num_end].parse::<i64>() else {
        return Ok(None);
    };
    let unit = &s[num_end..];

    let local_midnight = |date: NaiveDate| -> Result<taskchampion::chrono::DateTime<Utc>> {
        Ok(date
            .and_hms_opt(0, 0, 0)
            .expect("midnight valid")
            .and_local_timezone(Local)
            .single()
            .context("ambiguous local time")?
            .with_timezone(&Utc))
    };

    match unit {
        // Hours: wall-clock offset (minutes matter for deadlines)
        "h" | "hr" | "hrs" => Ok(Some(now + Duration::hours(n))),
        // Days/weeks/months: resolve to local midnight for consistency with named keywords
        "d" | "da" | "day" | "days" => Ok(Some(local_midnight(today_local + Duration::days(n))?)),
        "w" | "wk" | "wks" => Ok(Some(local_midnight(today_local + Duration::weeks(n))?)),
        // Approximate: N*30 days from today midnight
        "mo" => Ok(Some(local_midnight(today_local + Duration::days(n * 30))?)),
        _ => Ok(None),
    }
}

pub fn parse_kv(kv: &str) -> Result<(&str, &str)> {
    let Some(pos) = kv.find('=') else {
        anyhow::bail!("Invalid --set format {kv:?} — expected key=value");
    };
    Ok((&kv[..pos], &kv[pos + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use taskchampion::chrono::{Datelike, Duration, Local, Weekday};

    #[test]
    fn test_parse_date_absolute() {
        let dt = parse_date("2026-03-20").unwrap();
        // Verify round-trip: local date should match input
        assert_eq!(
            dt.with_timezone(&Local).date_naive().to_string(),
            "2026-03-20"
        );
    }

    #[test]
    fn test_parse_date_today_tomorrow_yesterday() {
        let today = Local::now().date_naive();
        let dt_today = parse_date("today").unwrap();
        assert_eq!(dt_today.with_timezone(&Local).date_naive(), today);

        let dt_tomorrow = parse_date("tomorrow").unwrap();
        assert_eq!(
            dt_tomorrow.with_timezone(&Local).date_naive(),
            today + Duration::days(1)
        );

        let dt_yesterday = parse_date("yesterday").unwrap();
        assert_eq!(
            dt_yesterday.with_timezone(&Local).date_naive(),
            today - Duration::days(1)
        );
    }

    #[test]
    fn test_parse_date_now() {
        let before = Utc::now();
        let dt = parse_date("now").unwrap();
        let after = Utc::now();
        assert!(dt >= before && dt <= after);
    }

    #[test]
    fn test_parse_date_relative_days() {
        // Days resolve to local midnight N days from today
        let today = Local::now().date_naive();
        let dt = parse_date("2days").unwrap();
        assert_eq!(
            dt.with_timezone(&Local).date_naive(),
            today + Duration::days(2)
        );
    }

    #[test]
    fn test_parse_date_relative_hours() {
        // Hours use wall-clock time (not local midnight)
        let before = Utc::now();
        let dt = parse_date("3h").unwrap();
        let expected = before + Duration::hours(3);
        // Allow 5s tolerance for CI
        assert!((dt - expected).num_seconds().abs() <= 5);
    }

    #[test]
    fn test_parse_date_relative_weeks() {
        // Weeks resolve to local midnight N weeks from today
        let today = Local::now().date_naive();
        let dt = parse_date("1wk").unwrap();
        assert_eq!(
            dt.with_timezone(&Local).date_naive(),
            today + Duration::weeks(1)
        );
    }

    #[test]
    fn test_parse_date_eom_is_last_day_of_month() {
        let dt = parse_date("eom").unwrap();
        let local_dt = dt.with_timezone(&Local);
        let today = Local::now();
        // Last day of month: next month's first day minus 1
        let (y, m) = if today.month() == 12 {
            (today.year() + 1, 1)
        } else {
            (today.year(), today.month() + 1)
        };
        use taskchampion::chrono::NaiveDate;
        let expected = NaiveDate::from_ymd_opt(y, m, 1)
            .unwrap()
            .pred_opt()
            .unwrap();
        assert_eq!(local_dt.date_naive(), expected);
    }

    #[test]
    fn test_parse_date_som_is_first_of_next_month() {
        let dt = parse_date("som").unwrap();
        let local_dt = dt.with_timezone(&Local);
        let today = Local::now();
        let (y, m) = if today.month() == 12 {
            (today.year() + 1, 1)
        } else {
            (today.year(), today.month() + 1)
        };
        assert_eq!(local_dt.year(), y);
        assert_eq!(local_dt.month(), m);
        assert_eq!(local_dt.day(), 1);
    }

    #[test]
    fn test_parse_date_soy_is_jan_1_next_year() {
        let dt = parse_date("soy").unwrap();
        let local_dt = dt.with_timezone(&Local);
        assert_eq!(local_dt.year(), Local::now().year() + 1);
        assert_eq!(local_dt.month(), 1);
        assert_eq!(local_dt.day(), 1);
    }

    #[test]
    fn test_parse_date_weekday_is_future() {
        let dt = parse_date("mon").unwrap();
        let local_dt = dt.with_timezone(&Local);
        assert_eq!(local_dt.weekday(), Weekday::Mon);
        assert!(local_dt.date_naive() > Local::now().date_naive());
    }

    #[test]
    fn test_parse_date_eod_is_today() {
        let dt = parse_date("eod").unwrap();
        let local_dt = dt.with_timezone(&Local);
        assert_eq!(local_dt.date_naive(), Local::now().date_naive());
    }

    #[test]
    fn test_parse_date_later() {
        let dt = parse_date("later").unwrap();
        assert_eq!(dt.date_naive().year(), 9999);
    }

    #[test]
    fn test_parse_date_case_insensitive() {
        // Should handle mixed case
        assert!(parse_date("Today").is_ok());
        assert!(parse_date("TOMORROW").is_ok());
        assert!(parse_date("Mon").is_ok());
    }

    #[test]
    fn test_parse_date_invalid() {
        assert!(parse_date("garbage").is_err());
        assert!(parse_date("").is_err());
        assert!(parse_date("2026-13-01").is_err());
    }
}
