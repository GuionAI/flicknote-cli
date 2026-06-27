FlickNote works with local and managed workspaces.
Managed workspaces support note and organization commands.
Local workspaces are required for file, editor, browser, sync, sign-in, and skill commands.
Run `flicknote <command> --help` for exact flags and examples.

Common workflows:
  flicknote add "Meeting notes" --project work
  flicknote upload file.pdf --project work
  flicknote find "keyword"
  flicknote detail <id> --tree
  flicknote content <id> --section <section-id>
  cat edit.md | flicknote modify <id>
  cat note.md | flicknote replace <id>

Use numeric note IDs from `flicknote list`.
