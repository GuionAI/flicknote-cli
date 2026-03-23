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

## List

```bash
flicknote list --project <name>
flicknote find "keyword"
flicknote find "keyword1" "keyword2"    # OR match
flicknote list --json
```

## Read

```bash
flicknote get <id>
flicknote get <id> --tree                     # heading structure with section IDs
flicknote get <id> --section <section-id>     # use ID from --tree output (e.g. 3K)
flicknote get <id> --json
flicknote get <id> --archived               # read an archived note
```

To target a section, first run `--tree` to see IDs, then use the ID:

```bash
flicknote get abc12345 --tree
# └─ # My Note
#    ├─ [3K] ## Summary
#    └─ [aZ] ## Details
# Note: H1 headings are not shown with IDs and cannot be targeted with --section
flicknote get abc12345 --section 3K
```

## Replace / Append

```bash
echo "new content" | flicknote replace <id>
echo "new content" | flicknote replace <id> --section <section-id>
echo "more content" | flicknote append <id>
```

For multiline content with special characters, use heredoc:

```bash
cat <<'EOF' | flicknote replace <id>
# Updated Note
Content with **markdown** and $variables
EOF
```

Mutating commands (`replace`, `remove`, `rename`, `insert`) print the updated `--tree` after making changes.

## Section Operations

```bash
flicknote remove <id> --section <section-id>
flicknote rename <id> --section <section-id> "New Name"
echo "content" | flicknote insert <id> --before <section-id>
echo "content" | flicknote insert <id> --after <section-id>
# IDs are 2-character base62 (0–9, A–Z, a–z) — run --tree to find them; H1 headings have no ID
```

## Open in Browser

```bash
flicknote open <id>    # open note in browser
```

## Archive

```bash
flicknote archive <id>
```

Never pipe flicknote content through sed/awk — use replace/insert instead.
