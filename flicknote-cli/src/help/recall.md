Human mode takes one query argument:

  flicknote recall "memory systems"
  flicknote recall "memory systems" --project work

The query is text supplied by a person or the current conversation. An explicit
empty query is valid and returns no candidates. The command prints at most the
daemon's bounded recall candidates; it does not read note bodies.

Codex command-hook mode reads one UserPromptSubmit event from stdin and writes
the hook JSON response to stdout:

  flicknote recall --hook < event.json

The event must have `hook_event_name: "UserPromptSubmit"` and a string
`prompt`. Prompt text stays on stdin and is not inserted into shell commands.
