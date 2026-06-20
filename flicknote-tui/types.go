package main

import "fmt"

// Note matches the JSON output of `flicknote list --json` and `flicknote find --json`.
// This is a subset of the full Rust Note struct — fields the TUI doesn't need
// (external_id, metadata, source, deleted_at) are omitted. Go's JSON decoder
// silently ignores unknown fields, so this is forward-compatible.
type Note struct {
	ID        *int    `json:"id"`
	UUID      string  `json:"uuid"`
	Type      string  `json:"type"` // "text", "voice", "link"
	Status    string  `json:"status"`
	Title     *string `json:"title"`
	Content   *string `json:"content"`
	Summary   *string `json:"summary"`
	IsFlagged *int    `json:"is_flagged"`
	ProjectID *string `json:"project_id"`
	CreatedAt *string `json:"created_at"`
	UpdatedAt *string `json:"updated_at"`
}

// NoteDetail matches `flicknote detail <id> --json` output.
// This is a different shape than Note — project is the resolved name, not ID.
type NoteDetail struct {
	ID        *int    `json:"id"`
	UUID      string  `json:"uuid"`
	Type      string  `json:"type"`
	Title     *string `json:"title"`
	Project   *string `json:"project"` // resolved project name, not ID
	Summary   *string `json:"summary"`
	Content   *string `json:"content"`
	CreatedAt *string `json:"created_at"`
	UpdatedAt *string `json:"updated_at"`
}

func (n Note) Ref() string {
	if n.ID != nil {
		return fmt.Sprintf("%d", *n.ID)
	}
	return n.UUID
}

func (d NoteDetail) DisplayID() string {
	if d.ID != nil {
		return fmt.Sprintf("%d", *d.ID)
	}
	return "pending"
}
