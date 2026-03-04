---
name: flicknote-cli
description: "FlickNote CLI for managing notes — add, list, get, edit, and organize by project"
---

# FlickNote CLI

Local-first note management. Notes are stored in a local SQLite database and synced to the cloud.

## Adding Notes

```bash
# Add a text note (title is auto-generated from content)
flicknote add "Meeting notes about API redesign"

# Add a URL (auto-detected as link note)
flicknote add "https://example.com/article"

# Add to a project (creates project if it doesn't exist)
flicknote add "Design doc draft" --project myproject

# Pipe content from stdin
echo "long content here" | flicknote add --project myproject
cat notes.md | flicknote add --project research
```

## Listing Notes

```bash
flicknote list                          # recent notes (default: 20)
flicknote list --project myproject      # notes in a project
flicknote list --search "API"           # search by title or content
flicknote list --type link              # filter by type (normal, voice, link)
flicknote list --limit 50               # more results
flicknote list --archived               # show archived notes
flicknote list --json                   # JSON output
```

## Reading Notes

```bash
# Get full note details
flicknote get abc12345

# See heading structure of a long note
flicknote get abc12345 --tree

# Extract a specific section by heading name
flicknote get abc12345 --section "Summary"

# JSON output
flicknote get abc12345 --json
```

## Editing Notes

```bash
# Replace a section by heading name (reads new content from stdin if omitted)
flicknote edit abc12345 --section "Summary" "New summary content"
echo "updated content" | flicknote edit abc12345 --section "Summary"

# Replace entire note content
flicknote replace abc12345 "Completely new content"
cat updated.md | flicknote replace abc12345

# Append to an existing note (adds with \n\n separator)
flicknote append abc12345 "Additional notes from today"
echo "more content" | flicknote append abc12345

# Remove a section by heading
flicknote remove abc12345 --section "Outdated Notes"

# Rename a section heading (preserves heading level and body)
flicknote rename abc12345 --section "Draft" "Final"

# Insert content before or after a section
flicknote insert abc12345 --before "Summary" "## Preface\nContext for this doc"
flicknote insert abc12345 --after "Findings" "## Analysis\nDeeper dive here"
```

**Warning: Don't pipe flicknote content through sed/awk.** Content with code blocks, backticks, `$`, or `\` gets silently corrupted by shell substitution. Instead:
- Use `flicknote edit` or `flicknote replace` with a heredoc for the new content
- Use `flicknote insert --before/--after` to add sections
- Use `flicknote remove` to delete sections

## Uploading Files

```bash
# Upload a file and create a file-type note
flicknote upload screenshot.png --project myproject
flicknote upload report.pdf
```

## Note Types

| Type | Created by | Description |
|------|-----------|-------------|
| `normal` | `flicknote add "text"` | Text note |
| `link` | `flicknote add "https://..."` | URL auto-detected |
| `file` | `flicknote upload <path>` | Uploaded file |
| `voice` | Mobile app | Voice memo (transcribed) |

Projects are created automatically by `--project` — no separate project creation needed.

## Not for Common Use

- `flicknote import` — migration tool for importing markdown files into FlickNote. One-time use, not part of regular workflow.
