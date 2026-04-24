# flicknote Quick Reference

## Add

```bash
flicknote add "content" --project <name>
flicknote add "https://example.com"           # auto-detected as link
```

For multiline content with special characters, use heredoc:

```bash
cat <<'EOF' | flicknote add --project <name>
# My Note
Some content with **markdown** and $variables
EOF
```

## List & Find

```bash
flicknote list --project <name>
flicknote find "keyword"
flicknote find "keyword1" "keyword2"    # OR match
flicknote list --json
flicknote count                         # count active notes
flicknote count --project <name>        # count in project
```

## Read

```bash
flicknote detail <id>
flicknote detail <id> --tree                  # heading structure with section IDs
flicknote detail <id> --section <section-id>  # use ID from --tree output (e.g. 3K)
flicknote detail <id> --json
flicknote detail <id> --archived              # read an archived note
flicknote content <id>                        # content-only with section IDs
flicknote content <id> --section <section-id>
flicknote content <id> --raw                  # pure markdown, no section ID annotations (safe for piping)
```

To target a section, first run `--tree` to see IDs, then use the ID:

```bash
flicknote detail abc12345 --tree
# └─ # My Note
#    ├─ [3K] ## Summary
#    └─ [aZ] ## Details
# Note: H1 headings are not shown with IDs and cannot be targeted with --section
flicknote detail abc12345 --section 3K
```

## Replace (overwrite)

> `flicknote replace <id>` overwrites the whole note or a whole section including its heading. Prefer `modify` for precision edits.

```bash
echo "new note content" | flicknote replace <id>                       # replace entire note body
echo "## New Heading
new body" | flicknote replace <id> --section <s> # replace whole section incl. heading
flicknote replace <id> --project <name>                                # metadata works here too
```

`--section` requires stdin to start with an ATX or setext heading. The heading level is capped at the original section's level so outlines don't skew.

## Modify (edit-mode + metadata)

> `flicknote modify <id>` does precision string-replace via `===BEFORE===`/`===AFTER===` blocks, plus metadata.

```bash
# Edit mode: exact-string replacement (fails on zero or multiple matches)
cat <<'EDIT' | flicknote modify <id>
===BEFORE===
old text (exactly as in the note, whitespace-sensitive)
===AFTER===
new text
EDIT

# Scope to a section
cat <<'EDIT' | flicknote modify <id> --section <section-id>
===BEFORE===
old text inside that section
===AFTER===
new text
EDIT

# Metadata only
flicknote modify <id> --project <name>
flicknote modify <id> --title "New Title"
flicknote modify <id> --flagged   # or --unflagged
```

Rules:
- **Exact match, whitespace-sensitive.** No fuzzy fallbacks.
- **Unique-match required.** If `BEFORE` matches 0 or >1 times, you get a clear error. Add surrounding context to disambiguate.
- **Single block per call.** Multiple `===BEFORE===`/`===AFTER===` pairs in one stdin → error. Run modify multiple times for multiple edits.
- **Append** is a different command: `echo "more" | flicknote append <id>`.

Mutating commands (`modify`, `delete`, `rename`, `insert`) print the updated `--tree` after making changes.

### Migration from legacy `modify`

| Old                                                           | New                                                 |
|---------------------------------------------------------------|-----------------------------------------------------|
| `cat x.md | flicknote modify <id>`                            | `cat x.md | flicknote replace <id>`                 |
| `echo body | flicknote modify <id> --section <s>`             | `echo "## Heading
body" | flicknote replace <id> --section <s>`   |
| `cat "## X
..." | flicknote modify <id> --section <s> --with-heading` | `echo "## X
..." | flicknote replace <id> --section <s>`   (heading always in stdin; --with-heading removed) |

`--with-heading` is removed. For `replace --section`, the heading is always required in stdin.

## Section Operations

```bash
flicknote delete <id> --section <section-id>
flicknote rename <id> --section <section-id> "New Name"
echo "content" | flicknote insert <id> --before <section-id>
echo "content" | flicknote insert <id> --after <section-id>
# IDs are 2-character base62 (0–9, A–Z, a–z) — run --tree to find them; H1 headings have no ID
```

## Open in Browser

```bash
flicknote open <id>    # open note in browser
```

## Delete & Restore

```bash
flicknote delete <id>
flicknote restore <id>
```

Never pipe flicknote content through sed/awk — use modify/insert instead.
