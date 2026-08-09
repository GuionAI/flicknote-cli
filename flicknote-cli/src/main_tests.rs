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
fn upload_command_parses() {
    assert!(Cli::try_parse_from(["flicknote", "upload", "file.pdf"]).is_ok());
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
fn replace_requires_section() {
    assert!(Cli::try_parse_from(["flicknote", "replace", "1"]).is_err());
    assert!(Cli::try_parse_from(["flicknote", "replace", "1", "--section", "a1"]).is_ok());
}

#[test]
fn replace_rejects_metadata_flags() {
    for flag in ["--project", "--flagged", "--unflagged"] {
        let mut argv = vec!["flicknote", "replace", "1", "--section", "a1", flag];
        if flag == "--project" {
            argv.push("work");
        }
        assert!(Cli::try_parse_from(argv).is_err(), "accepted {flag}");
    }
}

#[test]
fn mcp_subcommand_parses() {
    assert!(Cli::try_parse_from(["flicknote", "mcp"]).is_ok());
}
