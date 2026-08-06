---
name: flicknote
description: "FlickNote CLI for managing notes - add, find, detail, modify, and organize by project"
---

# FlickNote CLI

Use FlickNote to save and retrieve local-first notes from the command line.
Run `flicknote <command> --help` for exact flags and examples.

## Project Use

Use `--project <name>` when the note belongs to a known project. Follow the
user's project name if they provide one; otherwise omit `--project`.

## Common Commands

```bash
flicknote add "note text" --project <project>
cat note.md | flicknote add --project <project>
flicknote upload file.pdf --project <project>
flicknote find "keyword"
flicknote find "::topic::AI::person::瓜子"
flicknote topic list
flicknote entity list --type person
flicknote source <id>
flicknote source <id> 12:19
flicknote source <id> --info
flicknote list --project <project>
flicknote detail <id>
flicknote detail <id> --tree
flicknote share <id>
flicknote unshare <id>
flicknote project share <project-id>
flicknote project unshare <project-id>
flicknote content <id>
flicknote content <id> --section <section-id>
flicknote gateway request --method POST --path /web/v1/search --json '{"query":"rust"}'
```

Use the numeric short ID shown by `flicknote list`.

## Editing Rules

Use `modify` for precise edits or note metadata. Use `replace` only to replace
one complete section subtree.

```bash
cat <<'EDIT' | flicknote modify <id>
===BEFORE===
old text exactly as it appears
===AFTER===
new text
EDIT

cat section.md | flicknote replace <id> --section <section-id>
```

`modify` requires one exact, whitespace-sensitive `===BEFORE===` /
`===AFTER===` block. The match must be unique. Add surrounding context if the
text appears more than once.

`replace` requires `--section`, and stdin must start with a heading. It cannot
replace a whole note or change its project/flagged state. To replace a whole
note, archive the old note and create a new one. For section IDs, run
`flicknote detail <id> --tree`.

Mutating section commands print the updated tree after the change.

## MCP Server

`flicknote mcp` serves typed note, source, and project tools over local stdio.
Configure an MCP client to run `flicknote` with `args: ["mcp"]`. Content and
exact `before`/`after` edits are JSON fields, so MCP callers do not use shell
heredocs or edit-mode delimiters. Note tools use numeric short IDs and hide
internal UUIDs; project tools use project names. Use `note_get` for editable
content and `note_source` only for stored source data. The server supports only
the local PowerSync workspace. Note creation and note/project share or unshare
require the sync daemon; local reads and edits do not.

The read-only `gateway_web_search` and `gateway_web_fetch` tools use the current
FlickNote session to call the configured Gateway's fixed web endpoints. Search
accepts `query`; fetch accepts `url` and returns only `content` and `wordCount`.
They do not accept arbitrary Gateway paths or headers. The `url` passed to fetch
is validated by the Gateway's Browser Gateway boundary; it does not change the
configured Gateway origin used by the CLI.

## Gateway Requests

`flicknote gateway request` makes an authenticated request only to an absolute
path on the Gateway origin configured by FlickNote. It refreshes the local
session when needed and keeps credentials inside the process. Do not extract a
JWT from `session.json`.

```bash
flicknote gateway request --method POST --path /web/v1/search --json '{"query":"rust"}'
cat request.json | flicknote gateway request --method POST --path /llm/v1/chat/completions --json
```

The response body, including SSE, is forwarded to stdout. Status and errors go
to stderr. Full URLs, redirects, caller-supplied headers, and token output are
not supported.

## More Help

```bash
flicknote --help
flicknote add --help
flicknote upload --help
flicknote list --help
flicknote detail --help
flicknote content --help
flicknote modify --help
flicknote replace --help
flicknote project --help
flicknote keyterm --help
```
