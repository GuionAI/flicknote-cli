Local data commands require the FlickNote daemon. Start it with `flicknote daemon start`.
The daemon owns the local PowerSync database and remote synchronization.
`flicknote private-mcp` is a separate foreground PostgreSQL server; see
`flicknote private-mcp --help` and docs/private-mcp.md for required settings.
Run `flicknote <command> --help` for exact flags and examples.

Common workflows:
  flicknote add "Meeting notes" --project work
  flicknote upload file.pdf --project work
  flicknote import notes/ --project work
  flicknote find "keyword"
  flicknote find "::topic::AI::person::瓜子"
  flicknote recall "current need"
  flicknote recall --hook < user-prompt-submit.json
  flicknote topic list
  flicknote entity list --type person
  flicknote source <id>
  flicknote detail <id> --tree
  flicknote content <id> --section <section-id>
  flicknote write <id> < replacement.md
  flicknote modify <id> --project work
  flicknote submit <id>
  flicknote share <id>
  flicknote unshare <id>
  flicknote project share <project-id>
  flicknote project unshare <project-id>
  flicknote note route-project < routes.json

The Gateway command is for internal development and maintenance requests.
Use numeric note IDs from `flicknote list`.
