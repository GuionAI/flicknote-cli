Data commands require the FlickNote daemon. Start it with `flicknote sync start`.
The daemon owns the local PowerSync database and remote synchronization.
Run `flicknote <command> --help` for exact flags and examples.

Common workflows:
  flicknote add "Meeting notes" --project work
  flicknote upload file.pdf --project work
  flicknote find "keyword"
  flicknote find "::topic::AI::person::瓜子"
  flicknote topic list
  flicknote entity list --type person
  flicknote source <id>
  flicknote detail <id> --tree
  flicknote content <id> --section <section-id>
  flicknote share <id>
  flicknote unshare <id>
  flicknote project share <project-id>
  flicknote project unshare <project-id>
  flicknote gateway request --path /healthz
  cat edit.md | flicknote modify <id>
  cat section.md | flicknote replace <id> --section <section-id>
  flicknote mcp

Use numeric note IDs from `flicknote list`.
