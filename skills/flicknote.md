---
name: flicknote
description: "MCP-first interface for daemon-backed FlickNote notes and projects"
---

# FlickNote MCP

Use FlickNote MCP for normal note and project operations. The MCP schemas are
the source of truth for tool names, arguments, and result fields; do not
recreate them with shell commands or Gateway requests.

## Identifiers

Use the numeric short note ID returned by MCP. Do not substitute a UUID. Project
operations identify projects by their names.

## Exact edits

`note_modify` performs one exact, whitespace-sensitive `before`/`after`
replacement. The `before` text must occur exactly once. Include more surrounding
context when a match is ambiguous. Content-editing fields remain separate from
metadata fields, so a project or flagged-state change can be combined with an
exact edit when appropriate.

## Section scope

Section mutation tools operate on a complete section subtree, including its
heading and child sections. Read the section tree before replacing, inserting,
renaming, or deleting a section. A section replacement must supply the complete
replacement heading and subtree; section deletion is destructive.

## Lifecycle

Archiving is the normal soft-delete operation. Treat archive as destructive and
use restore only when the user explicitly wants the identified archived note
back. Do not assume processing or synchronization status is part of the public
note contract.

## Recommended flow

Discover with the topic/entity tools, list or find notes, read the selected note,
then apply the smallest exact or section-scoped mutation. Verify the result with
a follow-up read. The shell CLI remains for human and operational workflows;
Gateway is internal development/maintenance tooling, not the agent interface.

## More help

The installed MCP schemas define the available tools, parameters, and result
fields. Use them rather than duplicating a command reference here.
