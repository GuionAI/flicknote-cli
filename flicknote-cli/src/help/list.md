Examples:
  flicknote list
  flicknote list --project work
  flicknote list --no-project
  flicknote list --created-after 2026-09-24T04:00:00+08:00 --created-before 2026-09-25T04:00:00+08:00
  flicknote list --type link
  flicknote list --limit 50
  flicknote list --limit 50 --cursor 123
  flicknote list --archived
  flicknote list --json

List output shows the note ID used by detail/content/edit commands.
JSON output returns lightweight discovery items: public short note ID, type,
title, project UUID/name, metadata, topics, summary, content_bytes (UTF-8 bytes
of stored content), flagged state, and timestamps. It does not include full
note content or internal note UUIDs.

--created-after is inclusive and --created-before is exclusive. Both accept
RFC3339 timestamps. --project and --no-project cannot be combined.

Results are ordered by note ID from newest to oldest. To fetch the next page,
pass the final result's ID to --cursor.
