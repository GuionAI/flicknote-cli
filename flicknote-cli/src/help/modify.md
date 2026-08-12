`modify` changes note metadata for a human CLI workflow. Provide at least one
metadata option; `--project` may be combined with either flagged option.

Options:
  - `--project <name>` moves the note to a project.
  - `--flagged` marks the note as flagged.
  - `--unflagged` removes the flagged state.
  - `--flagged` and `--unflagged` cannot be used together.

Examples:
  flicknote modify 123 --project work
  flicknote modify 123 --project work --flagged
  flicknote modify 123 --unflagged

Content and section edits are available through the structured FlickNote MCP
interface; this command does not read replacement documents from stdin.
