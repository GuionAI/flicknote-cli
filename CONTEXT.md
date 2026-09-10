# FlickNote Recall

Language for connecting the current conversation to existing notes.

## Language

**Entity**:
A named person, company, location, or product identified in a note.
_Avoid_: Keyword, topic

**Topic**:
A subject assigned to a note, such as Memory Systems or Knowledge Management. It describes what the note concerns, rather than naming a person, company, location, or product.
_Avoid_: Entity, named object

**Recall query**:
The current message text used to look for connections to existing notes. It expresses the current need, but need not contain the names or subjects recorded on those notes.
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
