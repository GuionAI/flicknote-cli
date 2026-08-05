Edit mode reads one exact replacement block from stdin:
  ===BEFORE===
  old text exactly as it appears
  ===AFTER===
  new text

Rules:
  - Exact match, whitespace-sensitive.
  - Unique match required; add surrounding context if the text appears more than once.
  - Single block per call.
  - `--section` scopes the match to the full section, including its heading.
  - For a section overwrite, use `flicknote replace <id> --section <section-id>`.

Examples:
  flicknote modify 123 --project work
  flicknote modify 123 --flagged

Apply an exact replacement from stdin:
cat <<'EOF' | flicknote modify 123
===BEFORE===
old text exactly as it appears
===AFTER===
new text
EOF
