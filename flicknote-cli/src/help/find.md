Examples:
  flicknote find "API"
  flicknote find "API" "REST"
  flicknote find "::topic::AI::person::瓜子"
  flicknote find "keyword" --project work
  flicknote find "keyword" --created-after 2026-01-01T00:00:00Z --human
  flicknote find "keyword" --limit 50
  flicknote find "keyword" --json

Multiple keywords use OR matching across title, content, and summary. The
final Latin term uses prefix matching from two characters; a single Latin
character alone returns no results. CJK characters use indexed tokens.
Structured extraction filters use ::type::value pairs and are matched with
AND logic. Extraction-only searches may use --archived. Lexical keywords
cannot be combined with extraction filters or --archived. JSON output returns
dedicated search hits with type, UTF-8 content byte length, draft state, and
segmented snippets, without full note content, internal scores, or source data.
Active searches exclude drafts; archived extraction-only searches include
matching archived notes regardless of draft state.
Use --created-after and --created-before with RFC3339 timestamps to bound
creation time. --human excludes notes created through MCP.
