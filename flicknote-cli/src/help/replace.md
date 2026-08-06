`flicknote replace` overwrites one whole section subtree, including its heading.
For precision edits, use `flicknote modify <id>`.

Rules:
  - Content is read from stdin.
  - `--section` is required.
  - Stdin must start with a heading.
  - Section heading level is capped at the original section level.
  - Project and flagged metadata are changed with `flicknote modify`.
  - To replace a whole note, archive it and create a new note.

Examples:
cat <<'EOF' | flicknote replace 123 --section 3K
## New heading

Replace the selected section with this text.
EOF
