Examples:
  flicknote project list
  flicknote project add work
  flicknote project detail <project-id>
  flicknote project share <project-id>
  flicknote project unshare <project-id>
  flicknote project modify <project-id> --color "#FF5733"
  flicknote project modify <project-id> --description "What belongs in this project"
  flicknote project delete <project-id>

Use project names with note commands, for example `flicknote add "text" --project work`.

Project list JSON exposes nullable `description`; detail labels it `Description`.
Modify omission preserves it; `--description none` clears it. Obsolete project
`--summary` is invalid. Note summaries remain independent.
