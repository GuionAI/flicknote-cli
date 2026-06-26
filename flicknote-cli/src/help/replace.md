`flicknote replace` overwrites the whole note or a whole section.
For precision edits, use `flicknote modify <id>`.

Rules:
  - Content is read from stdin.
  - Without `--section`, stdin replaces the note body.
  - `--section` requires stdin to start with a heading.
  - Section heading level is capped at the original section level.

Examples:
  echo "new content" | flicknote replace 123
  cat note.md | flicknote replace 123
  printf '## New Heading\nnew body\n' | flicknote replace 123 --section 3K
