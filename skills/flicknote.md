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

# Content-only as pure markdown — no section ID annotations (safe for piping to sed/awk)
flicknote content abc12345 --raw

# See heading structure with section IDs
flicknote detail abc12345 --tree

# Extract a specific section — use ID from --tree or content output (e.g. 3K)
flicknote detail abc12345 --section 3K
flicknote content abc12345 --section 3K
flicknote content abc12345 --section 3K --raw   # raw section content, no ID annotation

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

Content-writing commands read from **stdin only** — pipe content in or use heredoc.

### Replace (overwrite)

> `flicknote replace <id>` overwrites the whole note or a whole section including its heading. Prefer `modify` for precision edits.

```bash
# Replace entire note content
echo "new content" | flicknote replace abc12345

# Replace a section (stdin MUST start with a heading — heading level is capped at original)
echo "## New Heading
new body" | flicknote replace abc12345 --section 3K
```

`--section` requires stdin to start with an ATX or setext heading.

### Modify (edit-mode + metadata)

> `flicknote modify <id>` does precision string-replace via `===BEFORE===`/`===AFTER===` blocks, plus metadata.

```bash
# Edit mode: exact-string replacement (fails on zero or multiple matches)
cat <<'EDIT' | flicknote modify abc12345
===BEFORE===
old text (exactly as in the note, whitespace-sensitive)
===AFTER===
new text
EDIT

# Scope to a section (scope = full section including heading)
cat <<'EDIT' | flicknote modify abc12345 --section 3K
===BEFORE===
old text inside that section
===AFTER===
new text
EDIT

# Metadata only
flicknote modify abc12345 --project newproject
flicknote modify abc12345 --title "New Title"
flicknote modify abc12345 --flagged
```

**Rules:**
- **Exact match, whitespace-sensitive.** No fuzzy fallbacks.
- **Unique-match required.** If `BEFORE` matches 0 or >1 times, you get a clear error. Add surrounding context to disambiguate.
- **Single block per call.** Multiple `===BEFORE===`/`===AFTER===` pairs in one stdin → error. Run modify multiple times.
- **Append** is a different command: `echo "more" | flicknote append <id>`

### Other content operations

```bash
# Append to an existing note (stdin required, adds with \n\n separator)
echo "more content" | flicknote append abc12345

# Remove a section by ID
flicknote delete abc12345 --section 3K

# Rename a section heading (preserves heading level and body)
flicknote rename abc12345 --section 3K "Final"

# Insert content before or after a section by ID
echo "## Preface" | flicknote insert abc12345 --before 3K
echo "## Analysis" | flicknote insert abc12345 --after aZ
```

Mutating commands print the updated `--tree` after making changes.

### Warning

**Don't pipe `flicknote content` output through sed/awk.** The section IDs (`[Xk]` annotations) will corrupt diffs. Use `--raw` to get pure markdown without annotations.

### Migration from legacy `modify`

| Old                                                           | New                                                 |
|---------------------------------------------------------------|-----------------------------------------------------|
| `cat x.md \| flicknote modify <id>`                            | `cat x.md \| flicknote replace <id>`                 |
| `echo body \| flicknote modify <id> --section <s>`             | `echo "## Heading
body" \| flicknote replace <id> --section <s>`   |
| `cat "## X
..." \| flicknote modify <id> --section <s> --with-heading` | `echo "## X
..." \| flicknote replace <id> --section <s>`   (heading always in stdin; --with-heading removed) |

`--with-heading` is removed.

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
