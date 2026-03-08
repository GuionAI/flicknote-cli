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
flicknote list --search "keyword"
flicknote list --json
```

## Read

```bash
flicknote get <id>
flicknote get <id> --tree                     # heading structure
flicknote get <id> --section "Section Name"
flicknote get <id> --json
```

## Replace / Append

```bash
echo "new content" | flicknote replace <id>
echo "new content" | flicknote replace <id> --section "Name"
echo "more content" | flicknote append <id>
```

For multiline content with special characters, use heredoc:

```bash
cat <<'EOF' | flicknote replace <id>
# Updated Note
Content with **markdown** and $variables
EOF
```

## Section Operations

```bash
flicknote remove <id> --section "Name"
flicknote rename <id> --section "Old" "New"
echo "content" | flicknote insert <id> --before "Section"
echo "content" | flicknote insert <id> --after "Section"
```

## Archive

```bash
flicknote archive <id>
```

Never pipe flicknote content through sed/awk — use replace/insert instead.
