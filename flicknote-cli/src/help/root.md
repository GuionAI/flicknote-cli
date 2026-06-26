FlickNote CLI — local-first note management.

Notes are stored locally and synced to the cloud.
Run `flicknote <command> --help` for exact flags and examples.

Common workflows:
  flicknote add "Meeting notes" --project orientation
  flicknote find "keyword"
  flicknote detail <id> --tree
  flicknote content <id> --section <section-id>
  cat edit.md | flicknote modify <id>
  cat note.md | flicknote replace <id>

Use numeric note IDs from `flicknote list`. Pending-sync notes may need a full UUID.
