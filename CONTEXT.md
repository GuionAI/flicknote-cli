# FlickNote Recall

Language for connecting the current conversation to existing notes.

## Language

**Entity**:
A named person, company, location, or product identified in a note.
_Avoid_: Keyword, topic

**Entity recall**:
Finding candidate notes when the current message contains the name of an entity associated with those notes. A match suggests a possible connection, not that the note answers the message.
_Avoid_: Semantic search, answer retrieval

**Recall candidate**:
An existing note offered for possible further reading, represented by its identifier, title, available summary, and modification time. It is historical material whose relevance and claims still need evaluation.
_Avoid_: Verified fact, instruction

**Note modification time**:
The time a note was last changed. It does not establish when the described event happened or whether a claim is more accurate.
_Avoid_: Event time, evidence of truth
