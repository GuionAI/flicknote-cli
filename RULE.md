# flicknote Quick Reference

## Add

```bash
flicknote add "content" --project <name>
cat file.md | flicknote add --project <name>
flicknote add "https://example.com"           # auto-detected as link
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

## Edit

```bash
flicknote edit <id> --section "Name" "new content"
echo "content" | flicknote edit <id> --section "Name"
flicknote replace <id> "full new content"
flicknote append <id> "additional content"
```

## Section Operations

```bash
flicknote remove <id> --section "Name"
flicknote rename <id> --section "Old" "New"
flicknote insert <id> --before "Section" "content"
flicknote insert <id> --after "Section" "content"
```

## Archive

```bash
flicknote archive <id>
```

Never pipe flicknote content through sed/awk — use edit/replace/insert instead.
