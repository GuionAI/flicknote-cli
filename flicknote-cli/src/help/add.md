Examples:
  flicknote add "Meeting notes about API redesign"
  flicknote add "https://example.com/article"
  flicknote add "Design doc draft" --project work

Create a multi-line note from stdin:
cat <<'EOF' | flicknote add --project work
# API redesign

Document the new endpoint contract.
EOF

Text and URLs are auto-detected. Use `flicknote upload <path>` for files.
