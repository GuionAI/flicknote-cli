# FlickNote

Language for note ownership, discovery, and connecting the current conversation to existing notes.

## Note ownership

**FlickNote plugin**:
The publicly discoverable FlickNote integration through which a person connects
their FlickNote account and works with their own notes in ChatGPT.
_Avoid_: CLI installer, shared note collection

**Connected FlickNote account**:
The FlickNote account a person has authorized the plugin to access. Publishing
the plugin does not make that account's notes public.
_Avoid_: OpenAI account, plugin publisher

**Draft note**:
A stored note that has not been submitted for processing. Editing its content
or metadata does not submit it.
_Avoid_: Unsaved note, queued note

**Note submission**:
The explicit transition that sends a draft note into processing. It is distinct
from creating a draft or editing an existing note.
_Avoid_: Save, content edit

**Note owner**:
The FlickNote user to whom a note belongs. A note's project assignment and the client used to access it do not change its owner.
_Avoid_: Project owner, MCP client, server operator

**Private note collection**:
The notes belonging to one FlickNote user, across all of that user's projects and notes without a project. Another user's authorization does not grant access to this collection.
_Avoid_: Project, shared workspace

## Language

**Entity**:
A named person, company, location, or product identified in a note.
_Avoid_: Keyword, topic

**Topic**:
A subject assigned to a note, such as Memory Systems or Knowledge Management. It describes what the note concerns, rather than naming a person, company, location, or product.
_Avoid_: Entity, named object

**Recall query**:
Text supplied by a person or the current conversation to look for connections to existing notes. It expresses the current need, but need not contain the names or subjects recorded on those notes.
_Avoid_: Extracted entity, answer

**Entity recall**:
Finding candidate notes when the current message contains the name of an entity associated with those notes. A match suggests a possible connection, not that the note answers the message.
_Avoid_: Semantic search, answer retrieval

**Topic recall**:
Finding candidate notes when the current message contains a subject associated with those notes. A match suggests a possible connection, not that the note answers the message.
_Avoid_: Semantic search, answer retrieval

**Recall candidate**:
An existing note offered for possible further reading, represented by its identifier, title, available summary, and modification time. It is historical material whose relevance and claims still need evaluation.
_Avoid_: Verified fact, instruction

**Note modification time**:
The time a note was last changed. It does not establish when the described event happened or whether a claim is more accurate.
_Avoid_: Event time, evidence of truth

## Note search

**Note search**:
An explicit request by a person or an agent to find existing notes from remembered words or text fragments. It returns candidates for inspection rather than asserting that a note answers the request.
_Avoid_: Recall, answer retrieval

**Human search**:
Note search initiated by a person. Its query may be approximate or contain spelling mistakes.
_Avoid_: Recall query

**Agent search**:
Note search initiated by an agent while carrying out a task. Its query is deliberate but the returned notes still require inspection before their contents become evidence.
_Avoid_: Automatic recall, verified evidence

**Match excerpt**:
A bounded passage from a note showing why a search result matched the query. It supports judging a result before opening the full note and is distinct from the note's summary.
_Avoid_: Summary, generated answer

## Note discovery

**FlickNote workspace**:
The desktop surface where a person captures, reads, and organizes their FlickNote notes. Closing its window is distinct from quitting the application.
_Avoid_: CLI window, MCP client

**Note-list page**:
A bounded, short-ID-descending segment of the active or archived notes selected by the same note-list filters. Its final item's short ID is the continuation cursor when another page is needed.
_Avoid_: Result array, batch

**Note-list cursor**:
The short ID from the final item of a note-list page. It selects later pages of the same list, whose notes have lower short IDs.
_Avoid_: Offset, page number, opaque token

## Note provenance

**MCP-created note**:
A note created through the FlickNote MCP interface, regardless of who wrote its content. Its `created_by_ai: true` metadata records the creation channel independently of client or session. Later edits and AI processing preserve this classification.
_Avoid_: AI-authored note

**Human-filtered note**:
A note whose `created_by_ai` metadata is absent or false. Human filters exclude only JSON boolean true, so other JSON values also remain included. This describes creation channel; it does not prove a person authored the content.
_Avoid_: Human-authored note

## Local application hosting

**Normal GUI host**:
The macOS workspace process owning the existing normal config/session/data,
PowerSync, Unix IPC and local MCP. It shares daemon config resolution and single
directory ownership; closing its window retains the host, explicit Quit ends it.
_Avoid_: Independent DEV profile, daemon client, automatic takeover

**Synthetic GUI mode**:
Explicit `--root` launch using fixture-owned storage and synthetic creation,
without normal configuration, sessions or cloud operations.
_Avoid_: Normal account, shared daemon directory
