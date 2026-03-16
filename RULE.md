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

## Archive

```bash
flicknote archive <id>
```

Never pipe flicknote content through sed/awk — use replace/insert instead.

# flicktask Quick Reference

## Add

```bash
flicktask add "description"
flicktask add "description" --parent a1b2c3d4 --priority H --due 2026-03-20
flicktask add "description" --tag backend --project auth
```

## List / Tree

```bash
flicktask list                     # all pending root tasks
flicktask list --tag backend       # filter by tag
flicktask list --due today         # due today
flicktask tree                     # full task tree
flicktask tree a1b2c3d4            # subtree of a task
```

## Get

```bash
flicktask get a1b2c3d4             # full task details + subtree
flicktask get a1b2c3d4 --depth 2
```

## Edit / Complete / Delete

```bash
flicktask edit a1b2c3d4 --description "New" --due 2026-04-01 --priority M
flicktask done a1b2c3d4
flicktask delete a1b2c3d4
flicktask undo
```

## Tags & Annotations

```bash
flicktask tag a1b2c3d4 backend
flicktask untag a1b2c3d4 backend
flicktask annotate a1b2c3d4 "Blocked by upstream API"
echo "Long note" | flicktask annotate a1b2c3d4
```

## Plan (markdown → subtask tree)

```bash
cat <<'EOF' | flicktask plan a1b2c3d4
## Research
Look into existing solutions.

## Implementation
Write the code.
EOF

cat plan.md | flicktask plan a1b2c3d4 --replace   # replace existing subtasks
```

## Move

```bash
flicktask move a1b2c3d4 b2c3d4e5   # reparent
flicktask move a1b2c3d4            # move to root
```

Task IDs are 8-character hex strings (e.g. `a1b2c3d4`).
