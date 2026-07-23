`flicknote replace` overwrites the whole note or a whole section.
For precision edits, use `flicknote modify <id>`.

Rules:
  - Content is read from stdin.
  - Without `--section`, stdin replaces the note body.
  - `--section` requires stdin to start with a heading.
  - Section heading level is capped at the original section level.

Examples:
cat <<'EOF' | flicknote replace 123
# Updated title

Replace the whole note body with this text.
EOF

cat <<'EOF' | flicknote replace 123 --section 3K
## New heading

Replace the selected section with this text.
EOF
