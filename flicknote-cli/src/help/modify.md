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
  - For overwrite, use `flicknote replace <id>`.

Examples:
  cat edit.md | flicknote modify 123
  cat edit.md | flicknote modify 123 --section 3K
  flicknote modify 123 --project work
  flicknote modify 123 --flagged
