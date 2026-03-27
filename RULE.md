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

## Modify / Append

```bash
echo "new content" | flicknote modify <id>
echo "new content" | flicknote modify <id> --section <section-id>          # body only, heading preserved
echo "## New Heading\ncontent" | flicknote modify <id> --section <section-id> --with-heading  # replaces heading too
# Without --with-heading: stdin must NOT start with # (errors if it does)
# With --with-heading: stdin MUST start with # (errors if it doesn't)
echo "more content" | flicknote append <id>
flicknote modify <id> --project <name>       # move to project
flicknote modify <id> --title "New Title"    # rename
flicknote modify <id> --flagged              # flag/unflag
```

For multiline content with special characters, use heredoc:

```bash
cat <<'EOF' | flicknote modify <id>
# Updated Note
Content with **markdown** and $variables
EOF
```

Mutating commands (`modify`, `delete`, `rename`, `insert`) print the updated `--tree` after making changes.

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
