---
name: flicknote
description: "FlickNote CLI for managing notes — add, list, get, and organize by project"
---

# FlickNote CLI

Local-first note management. Notes are stored in a local SQLite database and synced to the cloud.

## Quick Reference

```bash
# Add a text note
flicknote add "Meeting notes about API redesign"

# Add a URL (auto-detected as link note)
flicknote add "https://example.com/article"

# Add a note to a project (creates project if it doesn't exist)
flicknote add "Design doc draft" --project myproject

# Add a note linked to a taskwarrior task
flicknote add "Research findings" --task <uuid>

# List recent notes (default: 20)
flicknote list
flicknote list --limit 50

# List notes in a project
flicknote list --project myproject

# List notes linked to a taskwarrior task
flicknote list --task <uuid>

# Search notes by title
flicknote list --search "API"

# Filter by type (normal, voice, link)
flicknote list --type link

# Show archived notes
flicknote list --archived

# Get a note by ID (full UUID or short prefix)
flicknote get abc12345

# JSON output (works with list and get)
flicknote list --json
flicknote get abc12345 --json
```

Projects are created automatically by `--project` on `add` — no separate project creation needed.

## Common Patterns

**Capture a note for a task:**
```bash
flicknote add "Research: caching strategies for search-gateway" --project flicknote --task abc12345
```

**Browse notes in a project:**
```bash
flicknote list --project flicknote
```

**Get full content of a note:**
```bash
flicknote get abc12345
```

## Note Types

| Type | Created by | Description |
|------|-----------|-------------|
| `normal` | `flicknote add "text"` | Text note |
| `link` | `flicknote add "https://..."` | URL auto-detected |
| `voice` | Mobile app | Voice memo (transcribed) |

## Not for Common Use

- `flicknote import` — migration tool for importing markdown files into FlickNote. One-time use, not part of regular workflow.
