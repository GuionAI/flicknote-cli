---
name: flicknote
description: "FlickNote CLI for managing notes — add, list, detail, modify, and organize by project"
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

## Listing & Finding Notes

```bash
flicknote list                          # recent notes (default: 20)
flicknote list --project myproject      # notes in a project
flicknote find "API"                    # search by keyword (OR match)
flicknote find "API" "REST"             # multiple keywords
flicknote list --type link              # filter by type (normal, voice, link)
flicknote list --limit 50               # more results
flicknote list --archived               # show archived notes
flicknote list --json                   # JSON output
flicknote count                         # count active notes
flicknote count --project myproject     # count in project
flicknote count "API"                   # count by keyword filter
```

List columns: ID (full UUID) | Type | Title | Project | Topics | Flagged | Created

## Reading Notes

```bash
# Full metadata + content with section IDs
flicknote detail abc12345

# Content-only with section IDs after each heading
flicknote content abc12345

# See heading structure with section IDs
flicknote detail abc12345 --tree

# Extract a specific section — use ID from --tree or content output (e.g. 3K)
flicknote detail abc12345 --section 3K
flicknote content abc12345 --section 3K

# JSON output
flicknote detail abc12345 --json

# Read an archived note
flicknote detail abc12345 --archived

# Works with other flags
flicknote detail abc12345 --archived --tree
flicknote detail abc12345 --archived --json
```

Content output format:
```
# My Note [Xk]
## Summary [Fb]
...content...
## Key Points [eg]
...content...
```

To target a section, first run `--tree` to see IDs, then use the ID with `--section`:

```bash
flicknote detail abc12345 --tree
# └─ # My Note
#    ├─ [3K] ## Summary
#    └─ [aZ] ## Details
flicknote detail abc12345 --section 3K
```

## Editing Notes

All content-writing commands read from **stdin only** — pipe content in or use heredoc.

> **Warning:** `flicknote modify <id>` without `--section` **replaces the entire note content** with stdin. To edit only a section, always use `--section <id>`.

```bash
# Replace entire note content (stdin required)
echo "Completely new content" | flicknote modify abc12345  # ⚠️ replaces ENTIRE note
cat updated.md | flicknote modify abc12345

# Replace a section by ID (stdin = body only, heading is preserved)
# stdin must NOT start with # — errors if it does (use --with-heading instead)
echo "updated body content" | flicknote modify abc12345 --section 3K

# Replace a section including new heading (stdin MUST start with #)
echo "## New Name\nupdated content" | flicknote modify abc12345 --section 3K --with-heading

# Append to an existing note (stdin required, adds with \n\n separator)
echo "more content" | flicknote append abc12345

# Remove a section by ID (use delete --section)
flicknote delete abc12345 --section 3K

# Rename a section heading (preserves heading level and body)
flicknote rename abc12345 --section 3K "Final"

# Insert content before or after a section by ID (stdin required)
echo "## Preface\nContext for this doc" | flicknote insert abc12345 --before 3K
echo "## Analysis\nDeeper dive here" | flicknote insert abc12345 --after aZ
```

Mutating commands print the updated `--tree` after making changes, so you can see new IDs without a separate call.

**Warning: Don't pipe flicknote content through sed/awk.** Content with code blocks, backticks, `$`, or `\` gets silently corrupted by shell substitution. Instead:
- Use `flicknote modify` with a heredoc for the new content
- Use `flicknote insert --before/--after` to add sections
- Use `flicknote delete --section` to remove sections

## Modifying Note Metadata

```bash
flicknote modify abc12345 --project newproject   # move note to a different project
flicknote modify abc12345 --title "New Title"    # rename the note
flicknote modify abc12345 --flagged              # flag the note
flicknote modify abc12345 --unflagged            # unflag the note

# Combine content replacement with metadata in one call
cat updated.md | flicknote modify abc12345 --project newproject --flagged
```

Projects are created automatically if they don't exist. An empty project is deleted automatically after the last note is moved out.

## Opening Notes in Browser

```bash
flicknote open abc12345    # open note in browser
```

## Deleting & Restoring Notes

```bash
flicknote delete abc12345      # soft-delete a note (hidden from normal listing)
flicknote restore abc12345     # restore a deleted note
flicknote list --archived      # list deleted notes
```

## Projects

```bash
flicknote project list                  # list projects
flicknote project add myproject         # create a project
flicknote project detail abc12345       # show project details
flicknote project modify abc12345 --prompt <uuid>    # associate a prompt
flicknote project modify abc12345 --keyterm <uuid>   # associate keyterms
flicknote project modify abc12345 --color "#FF5733"  # set color
flicknote project modify abc12345 --prompt none      # clear prompt
flicknote project delete abc12345       # archive/delete a project
```

## Prompts

```bash
flicknote prompt add --title "My Prompt" --prompt "You are a ..."
flicknote prompt list
flicknote prompt detail abc12345
flicknote prompt modify abc12345 --title "New Title"
flicknote prompt delete abc12345
```

## Keyterms

```bash
flicknote keyterm add --name "My Terms" --content "term1: definition\nterm2: definition"
flicknote keyterm list
flicknote keyterm detail abc12345
flicknote keyterm modify abc12345 --content "updated content"
flicknote keyterm delete abc12345
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

## Not for Common Use

- `flicknote import` — migration tool for importing markdown files into FlickNote. One-time use, not part of regular workflow.
