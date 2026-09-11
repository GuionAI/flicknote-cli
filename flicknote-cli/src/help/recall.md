Human mode takes one query argument:

  flicknote recall "memory systems"
  flicknote recall "memory systems" --project work

The query is text supplied by a person or the current conversation. An explicit
empty query is valid and returns no candidates. The command prints at most the
daemon's bounded recall candidates; it does not read note bodies.
Human recall allows five seconds for the complete daemon call, including the
IPC connection and response.

Codex command-hook mode reads one UserPromptSubmit event from stdin and writes
the hook JSON response to stdout:

  flicknote recall --hook < event.json

The event must have `hook_event_name: "UserPromptSubmit"` and a string
`prompt`. Prompt text stays on stdin and is not inserted into shell commands.
Hook recall and MCP `note_recall` allow three seconds for their complete daemon
call. The installed Codex command hook also has a three-second host timeout,
which bounds an input stream that never reaches EOF. Data commands require the
FlickNote daemon; start it with `flicknote daemon start` when it is unavailable.
A response timeout is reported as a timeout rather than daemon unavailability:
human recall exits nonzero with a diagnostic, while hook failures write no
context to stdout so the host can continue without recalled notes.

After upgrading, explicitly reinstall an existing command hook with
`flicknote hook install codex --local` or `--global` to write the new
three-second host timeout. The extraction-first query is daemon-side, so an
updated daemon must be running before the database improvement is used. The
synthetic measurements are comparative evidence, not a universal latency SLA.
