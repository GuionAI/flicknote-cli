package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os/exec"
	"strings"
)

// Client shells out to the flicknote CLI for data access.
type Client struct{}

// runJSON runs flicknote with the given args, captures stdout, and unmarshals
// JSON into T. Stderr is captured and included in the error when non-empty.
func runJSON[T any](args ...string) (T, error) {
	var zero T
	out, err := exec.Command("flicknote", args...).Output()
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) && len(exitErr.Stderr) > 0 {
			return zero, fmt.Errorf("flicknote %s: %w\n%s", args[0], err, strings.TrimSpace(string(exitErr.Stderr)))
		}
		return zero, fmt.Errorf("flicknote %s: %w", args[0], err)
	}
	var result T
	if err := json.Unmarshal(out, &result); err != nil {
		return zero, fmt.Errorf("parse response: %w", err)
	}
	return result, nil
}

// ListNotes fetches all notes. Uses --limit 500 since the CLI defaults to 20.
func (c *Client) ListNotes() ([]Note, error) {
	return runJSON[[]Note]("list", "--limit", "500", "--json")
}

// ListNotesForProject fetches notes filtered by project name.
func (c *Client) ListNotesForProject(project string) ([]Note, error) {
	return runJSON[[]Note]("list", "--project", project, "--limit", "500", "--json")
}

// SearchNotes searches notes by keywords (OR match).
func (c *Client) SearchNotes(keywords ...string) ([]Note, error) {
	args := append([]string{"find"}, keywords...)
	args = append(args, "--json")
	return runJSON[[]Note](args...)
}

// GetNote fetches a single note's detail by ID.
func (c *Client) GetNote(id string) (*NoteDetail, error) {
	note, err := runJSON[NoteDetail]("get", id, "--json")
	if err != nil {
		return nil, err
	}
	return &note, nil
}

// ArchiveNote soft-deletes a note.
func (c *Client) ArchiveNote(id string) error {
	cmd := exec.Command("flicknote", "archive", id)
	out, err := cmd.Output()
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) && len(exitErr.Stderr) > 0 {
			return fmt.Errorf("flicknote archive: %w\n%s", err, strings.TrimSpace(string(exitErr.Stderr)))
		}
		// Output() captures stdout; for archive, any stdout output is diagnostic
		if len(out) > 0 {
			return fmt.Errorf("flicknote archive: %w\n%s", err, strings.TrimSpace(string(out)))
		}
		return fmt.Errorf("flicknote archive: %w", err)
	}
	return nil
}
