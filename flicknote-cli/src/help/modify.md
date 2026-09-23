`modify` patches note metadata. Provide at least one metadata option; ordinary
metadata edits preserve lifecycle status and never trigger AI processing.

Options:
  - `--project <name>` moves the note to a project.
  - `--clear-project` removes the note from its project.
  - `--title <text>` and `--clear-title` set or clear the title.
  - `--summary <text>` and `--clear-summary` set or clear the summary.
  - `--flagged` marks the note as flagged.
  - `--unflagged` removes the flagged state.
  - `--flagged` and `--unflagged` cannot be used together.

Examples:
  flicknote modify 123 --project work
  flicknote modify 123 --title "Updated title" --summary "Short summary"
  flicknote modify 123 --clear-project --clear-summary
  flicknote modify 123 --project work --flagged
  flicknote modify 123 --unflagged

Use `flicknote write` for a machine full-content replacement. Structured section
edits are available through the FlickNote MCP interface.
