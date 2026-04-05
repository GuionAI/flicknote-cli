package main

// Note matches the JSON output of `flicknote list --json` and `flicknote find --json`.
// This is a subset of the full Rust Note struct — fields the TUI doesn't need
// (external_id, metadata, source, deleted_at) are omitted. Go's JSON decoder
// silently ignores unknown fields, so this is forward-compatible.
type Note struct {
	ID        string  `json:"id"`
	UserID    string  `json:"user_id"`
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

// NoteDetail matches `flicknote get <id> --json` output (custom 8-field object).
// This is a different shape than Note — project is the resolved name, not ID.
type NoteDetail struct {
	ID        string  `json:"id"`
	Type      string  `json:"type"`
	Title     *string `json:"title"`
	Project   *string `json:"project"` // resolved project name, not ID
	Summary   *string `json:"summary"`
	Content   *string `json:"content"`
	CreatedAt *string `json:"created_at"`
	UpdatedAt *string `json:"updated_at"`
}
