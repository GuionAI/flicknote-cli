---
name: flicknote
description: "FlickNote CLI for managing notes — add, list, get, replace, and organize by project"
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

# Heredoc for multiline content with special characters ($, backticks, \)
cat <<'EOF' | flicknote add --project myproject
# My Note
Content with **markdown** and $variables safely handled
EOF
```

## Listing Notes

```bash
flicknote list                          # recent notes (default: 20)
flicknote list --project myproject      # notes in a project
flicknote find "API"                    # search by keyword (OR match)
flicknote find "API" "REST"             # multiple keywords
flicknote list --type link              # filter by type (normal, voice, link)
flicknote list --limit 50               # more results
flicknote list --archived               # show archived notes
flicknote list --json                   # JSON output
```

## Reading Notes

```bash
# Get full note details
flicknote get abc12345

# See heading structure with section IDs
flicknote get abc12345 --tree

# Extract a specific section — use ID from --tree output (e.g. 3K)
flicknote get abc12345 --section 3K

# JSON output
flicknote get abc12345 --json
```

To target a section, first run `--tree` to see IDs, then use the ID with `--section`:

```bash
flicknote get abc12345 --tree
# └─ # My Note
#    ├─ [3K] ## Summary
#    └─ [aZ] ## Details
# Note: H1 headings are not shown with IDs and cannot be targeted with --section
flicknote get abc12345 --section 3K
```

## Editing Notes

All content-writing commands read from **stdin only** — pipe content in or use heredoc.

```bash
# Replace entire note content (stdin required)
echo "Completely new content" | flicknote replace abc12345
cat updated.md | flicknote replace abc12345

# Replace a section by ID (run --tree first to get the ID)
echo "updated content" | flicknote replace abc12345 --section 3K

# Append to an existing note (stdin required, adds with \n\n separator)
echo "more content" | flicknote append abc12345

# Remove a section by ID
flicknote remove abc12345 --section 3K

# Rename a section heading (preserves heading level and body)
flicknote rename abc12345 --section 3K "Final"

# Insert content before or after a section by ID (stdin required)
echo "## Preface\nContext for this doc" | flicknote insert abc12345 --before 3K
echo "## Analysis\nDeeper dive here" | flicknote insert abc12345 --after aZ
```

Mutating commands (`replace`, `remove`, `rename`, `insert`) print the updated `--tree` after making changes, so you can see new IDs without a separate `--tree` call.

**Warning: Don't pipe flicknote content through sed/awk.** Content with code blocks, backticks, `$`, or `\` gets silently corrupted by shell substitution. Instead:
- Use `flicknote replace` with a heredoc for the new content
- Use `flicknote insert --before/--after` to add sections
- Use `flicknote remove` to delete sections

## Moving Notes Between Projects

```bash
flicknote modify abc12345 --project newproject   # move note to a different project
```

Projects are created automatically if they don't exist. An empty project is deleted automatically after the last note is moved out.

## Archiving Notes

```bash
flicknote archive abc12345     # archive a note (soft-delete, hidden from normal listing)
flicknote unarchive abc12345   # restore an archived note
flicknote list --archived      # list archived notes
```

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
