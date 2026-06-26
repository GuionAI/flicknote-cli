Examples:
  flicknote add "Meeting notes about API redesign"
  flicknote add "https://example.com/article"
  flicknote add "Design doc draft" --project orientation
  cat notes.md | flicknote add --project research
  flicknote add screenshot.png --project research

Text and URLs are auto-detected. Readable text files are imported as text notes.
Other supported files are uploaded and create file-type notes.
