Examples:
  flicknote add "Meeting notes about API redesign"
  flicknote add "https://example.com/article"
  flicknote add "Design doc draft" --project work
  cat notes.md | flicknote add --project work
  flicknote add screenshot.png --project work

Text and URLs are auto-detected. Readable text files are imported as text notes.
Other supported files are uploaded and create file-type notes.
