Replace a note's stored content from standard input. This machine-oriented
operation preserves title, summary, project, flagged state, topics, and
lifecycle status. Empty input is rejected.

Examples:
  printf '%s\n' 'Replacement body' | flicknote write 123
  cat document.md | flicknote write 123 --json
