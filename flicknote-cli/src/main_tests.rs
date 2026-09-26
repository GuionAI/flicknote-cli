use clap::Parser;

use super::Cli;

mod mcp;

#[test]
fn detail_rejects_section_flag() {
    assert!(Cli::try_parse_from(["flicknote", "detail", "abc123", "--section", "a1"]).is_err());
}

#[test]
fn content_rejects_raw_flag() {
    assert!(Cli::try_parse_from(["flicknote", "content", "abc123", "--raw"]).is_err());
}

#[test]
fn skill_install_command_parses() {
    assert!(Cli::try_parse_from(["flicknote", "skill", "install"]).is_ok());
}

#[test]
fn note_share_command_parses() {
    assert!(Cli::try_parse_from(["flicknote", "share", "123"]).is_ok());
}

#[test]
fn project_share_command_parses() {
    assert!(
        Cli::try_parse_from([
            "flicknote",
            "project",
            "share",
            "550e8400-e29b-41d4-a716-446655440000",
        ])
        .is_ok()
    );
}

#[test]
fn note_unshare_command_parses() {
    assert!(Cli::try_parse_from(["flicknote", "unshare", "123"]).is_ok());
}

#[test]
fn shared_list_accepts_json_and_rejects_archive() {
    assert!(Cli::try_parse_from(["flicknote", "list", "--shared", "--json"]).is_ok());
    let error = Cli::try_parse_from(["flicknote", "list", "--shared", "--archived"])
        .err()
        .unwrap();
    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn project_unshare_command_parses() {
    assert!(
        Cli::try_parse_from([
            "flicknote",
            "project",
            "unshare",
            "550e8400-e29b-41d4-a716-446655440000",
        ])
        .is_ok()
    );
}

#[test]
fn project_modify_supports_summary_but_not_pinned() {
    let id = "550e8400-e29b-41d4-a716-446655440000";
    assert!(
        Cli::try_parse_from([
            "flicknote",
            "project",
            "modify",
            id,
            "--summary",
            "Boundary"
        ])
        .is_ok()
    );
    assert!(
        Cli::try_parse_from(["flicknote", "project", "modify", id, "--pinned", "true"]).is_err()
    );
}

#[test]
fn upload_command_parses() {
    assert!(Cli::try_parse_from(["flicknote", "upload", "file.pdf"]).is_ok());
}

#[test]
fn list_project_and_no_project_are_mutually_exclusive() {
    let Err(error) =
        Cli::try_parse_from(["flicknote", "list", "--project", "work", "--no-project"])
    else {
        panic!("project and no-project should conflict")
    };
    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn list_time_filters_and_route_project_parse() {
    assert!(
        Cli::try_parse_from([
            "flicknote",
            "list",
            "--created-after",
            "2026-09-24T00:00:00Z",
            "--created-before",
            "2026-09-25T00:00:00Z",
            "--no-project"
        ])
        .is_ok()
    );
    assert!(Cli::try_parse_from(["flicknote", "note", "route-project"]).is_ok());
}

#[test]
fn list_accepts_only_canonical_note_statuses() {
    for status in ["draft", "ai_queued", "source_queued", "ready"] {
        assert!(Cli::try_parse_from(["flicknote", "list", "--status", status]).is_ok());
    }
    assert!(Cli::try_parse_from(["flicknote", "list", "--status", "reday"]).is_err());
    assert!(Cli::try_parse_from(["flicknote", "list", "--status", "synced"]).is_err());
}

#[test]
fn metadata_discovery_and_source_commands_parse() {
    assert!(Cli::try_parse_from(["flicknote", "topic", "list"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "entity", "list", "--type", "person"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "source", "42"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "source", "42", "12:19"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "source", "42", "--json"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "source", "42", "--info"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "find", "::topic::AI::person::瓜子"]).is_ok());
}

#[test]
fn note_type_filters_accept_meeting_and_reject_voice() {
    for command in ["list", "count"] {
        assert!(Cli::try_parse_from(["flicknote", command, "--type", "meeting"]).is_ok());
        assert!(Cli::try_parse_from(["flicknote", command, "--type", "voice"]).is_err());
    }
}

#[test]
fn agent_content_mutation_commands_are_not_cli_commands() {
    for command in ["replace", "insert", "rename"] {
        assert!(
            Cli::try_parse_from(["flicknote", command, "1"]).is_err(),
            "accepted removed command {command}"
        );
    }
    assert!(Cli::try_parse_from(["flicknote", "delete", "1", "--section", "a1"]).is_err());
}

#[test]
fn modify_requires_metadata_and_accepts_valid_combinations() {
    assert!(Cli::try_parse_from(["flicknote", "modify", "1"]).is_err());
    assert!(Cli::try_parse_from(["flicknote", "modify", "1", "--project", "work"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "modify", "1", "--flagged"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "modify", "1", "--unflagged"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "modify", "1", "--title", "New"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "modify", "1", "--clear-title"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "modify", "1", "--summary", "Short"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "modify", "1", "--clear-summary"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "modify", "1", "--clear-project"]).is_ok());
    assert!(
        Cli::try_parse_from(["flicknote", "modify", "1", "--project", "work", "--flagged"]).is_ok()
    );
    assert!(Cli::try_parse_from(["flicknote", "modify", "1", "--flagged", "--unflagged"]).is_err());
    assert!(
        Cli::try_parse_from([
            "flicknote",
            "modify",
            "1",
            "--title",
            "New",
            "--clear-title"
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "flicknote",
            "modify",
            "1",
            "--summary",
            "Short",
            "--clear-summary"
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "flicknote",
            "modify",
            "1",
            "--project",
            "work",
            "--clear-project"
        ])
        .is_err()
    );
}

#[test]
fn draft_add_write_and_submit_commands_parse_without_a_status_setter() {
    assert!(Cli::try_parse_from(["flicknote", "add", "Body", "--draft", "--json"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "write", "1", "--json"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "submit", "1", "--json"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "add", "Body", "--status", "draft"]).is_err());
}

#[test]
fn modify_rejects_content_editing_and_section_arguments() {
    for argument in ["--before", "--after", "--section"] {
        let mut argv = vec!["flicknote", "modify", "1", "--project", "work", argument];
        argv.push("value");
        assert!(Cli::try_parse_from(argv).is_err(), "accepted {argument}");
    }
}

#[test]
fn daemon_command_family_parses_and_legacy_sync_is_rejected() {
    for command in ["install", "uninstall", "start", "stop", "restart", "run"] {
        assert!(
            Cli::try_parse_from(["flicknote", "daemon", command]).is_ok(),
            "daemon {command} should parse"
        );
    }
    assert!(Cli::try_parse_from(["flicknote", "daemon", "status"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "daemon", "status", "--verbose"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "daemon", "status", "--json"]).is_ok());
    assert!(
        Cli::try_parse_from(["flicknote", "daemon", "logs", "--lines", "25", "--follow"]).is_ok()
    );
    assert!(Cli::try_parse_from(["flicknote", "sync", "start"]).is_err());
}

#[test]
fn logout_force_option_parses() {
    assert!(Cli::try_parse_from(["flicknote", "logout", "--force"]).is_ok());
}

#[test]
fn mcp_subcommand_parses() {
    assert!(Cli::try_parse_from(["flicknote", "mcp"]).is_ok());
}

#[test]
fn recall_requires_one_positional_query_unless_hook_mode_is_selected() {
    assert!(Cli::try_parse_from(["flicknote", "recall", "Ada"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "recall", ""]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "recall", "--hook"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "recall", "--hook", "--project", "work"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "recall"]).is_err());
    assert!(Cli::try_parse_from(["flicknote", "recall", "Ada", "--hook"]).is_err());
    assert!(Cli::try_parse_from(["flicknote", "recall", "--stdin"]).is_err());
}

#[test]
fn codex_hook_install_command_parses_and_scope_flags_conflict() {
    assert!(Cli::try_parse_from(["flicknote", "hook", "install", "codex"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "hook", "install", "codex", "--local"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "hook", "install", "codex", "--global"]).is_ok());
    assert!(
        Cli::try_parse_from([
            "flicknote",
            "hook",
            "install",
            "codex",
            "--local",
            "--global"
        ])
        .is_err()
    );
}
