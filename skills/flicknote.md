---
name: flicknote
description: "FlickNote CLI for managing notes - add, find, detail, modify, and organize by project"
---

# FlickNote CLI

Use FlickNote to save and retrieve local-first notes from the command line.
Run `flicknote <command> --help` for exact flags and examples.

## Agent Defaults

Use only these projects for agent-written notes unless the user asks otherwise:

- `orientation` - plans, task context, design decisions, implementation strategy
- `research` - findings, references, discoveries, reusable knowledge

## Common Commands

```bash
flicknote add "note text" --project orientation
cat findings.md | flicknote add --project research
flicknote find "keyword"
flicknote list --project research
flicknote detail <id>
flicknote detail <id> --tree
flicknote content <id>
flicknote content <id> --section <section-id>
```

Use the numeric short ID shown by `flicknote list`. Pending-sync notes use the
shown UUID prefix until the numeric ID appears.

## Editing Rules

Prefer `modify` for precise edits and `replace` for overwrite.

```bash
cat <<'EDIT' | flicknote modify <id>
===BEFORE===
old text exactly as it appears
===AFTER===
new text
EDIT

cat note.md | flicknote replace <id>
```

`modify` requires one exact, whitespace-sensitive `===BEFORE===` /
`===AFTER===` block. The match must be unique. Add surrounding context if the
text appears more than once.

`replace` overwrites the whole note or section. With `--section`, stdin must
start with a heading. For section IDs, run `flicknote detail <id> --tree`.

Mutating section commands print the updated tree after the change.

## More Help

```bash
flicknote --help
flicknote add --help
flicknote list --help
flicknote detail --help
flicknote content --help
flicknote modify --help
flicknote replace --help
flicknote project --help
flicknote prompt --help
flicknote keyterm --help
```
