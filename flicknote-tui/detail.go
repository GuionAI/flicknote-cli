package main

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/glamour"
)

func renderMarkdown(content string, width int) string {
	r, err := glamour.NewTermRenderer(
		glamour.WithAutoStyle(),
		glamour.WithWordWrap(width),
	)
	if err != nil {
		return content // fallback to raw
	}
	rendered, err := r.Render(content)
	if err != nil {
		return content
	}
	return rendered
}

// buildDetailContent constructs the full detail text (metadata + content).
// Called from loadDetail to pre-split lines stored in the model.
func buildDetailContent(d *NoteDetail, renderedMD string) string {
	var b strings.Builder

	b.WriteString("ID:       " + d.ID + "\n")
	b.WriteString("Type:     " + d.Type + "\n")
	if d.Project != nil {
		b.WriteString("Project:  " + *d.Project + "\n")
	}
	if d.CreatedAt != nil {
		b.WriteString("Created:  " + *d.CreatedAt + "\n")
	}
	if d.UpdatedAt != nil {
		b.WriteString("Updated:  " + *d.UpdatedAt + "\n")
	}

	if d.Summary != nil && *d.Summary != "" {
		b.WriteString("\n── Summary ──\n")
		b.WriteString(*d.Summary + "\n")
	}

	if renderedMD != "" {
		b.WriteString("\n── Content ──\n")
		b.WriteString(renderedMD)
	}

	return b.String()
}

func (m Model) viewDetail() string {
	var b strings.Builder

	// Error takes over the view — detail may have failed to load
	if m.err != nil {
		titleBar := titleStyle.Width(m.width).Render(" FlickNote")
		b.WriteString(titleBar)
		b.WriteString("\n")
		b.WriteString(errorStyle.Render(fmt.Sprintf(" Error: %v", m.err)))
		b.WriteString("\n")
		status := " q/esc back"
		b.WriteString(statusBarStyle.Width(m.width).Render(status))
		return b.String()
	}

	if m.detail == nil {
		return ""
	}

	// Title bar
	icon := typeIcon(m.detail.Type)
	title := "(untitled)"
	if m.detail.Title != nil {
		title = *m.detail.Title
	}
	titleBar := titleStyle.Width(m.width).Render(fmt.Sprintf(" %s %s", icon, title))
	b.WriteString(titleBar)
	b.WriteString("\n")

	// Scrollable content — use pre-split lines stored in model (no per-frame rebuild)
	contentLines := m.detailContent
	visibleHeight := m.height - 3

	end := m.scrollOffset + visibleHeight
	if end > len(contentLines) {
		end = len(contentLines)
	}

	start := m.scrollOffset
	if start > len(contentLines) {
		start = len(contentLines)
	}

	for i := start; i < end; i++ {
		b.WriteString(" " + contentLines[i] + "\n")
	}

	// Pad
	rendered := end - start
	for i := rendered; i < visibleHeight; i++ {
		b.WriteString("\n")
	}

	// Status bar
	status := " j/k scroll  │  C-d/C-u half-page  │  g top  │  G bottom  │  q/esc back"
	b.WriteString(statusBarStyle.Width(m.width).Render(status))

	return b.String()
}
