Examples:
  flicknote list
  flicknote list --project work
  flicknote list --type link
  flicknote list --limit 50
  flicknote list --archived
  flicknote list --json

List output shows the note ID used by detail/content/edit commands.
JSON output returns lightweight discovery items: note ID, type, title, project,
topics, summary, flagged state, and timestamps. It does not include full note
content or internal UUIDs.
