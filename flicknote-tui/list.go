package main

import (
	"fmt"
	"strings"
)

func (m Model) viewList() string {
	var b strings.Builder

	// Title bar
	title := " FlickNote"
	if m.searchQuery != "" {
		title = fmt.Sprintf(" FlickNote — search: %q", m.searchQuery)
	}
	if m.project != "" {
		title += fmt.Sprintf(" [%s]", m.project)
	}
	titleBar := titleStyle.Width(m.width).Render(title)
	b.WriteString(titleBar)
	b.WriteString("\n")

	if m.err != nil {
		b.WriteString(errorStyle.Render(fmt.Sprintf(" Error: %v", m.err)))
		b.WriteString("\n")
	}

	// Note list (offset is pre-computed in Update via adjustListOffset)
	h := m.listHeight()

	end := m.offset + h
	if end > len(m.notes) {
		end = len(m.notes)
	}

	for i := m.offset; i < end; i++ {
		note := m.notes[i]
		icon := typeIcon(note.Type)
		title := noteTitle(note)
		date := formatDate(note.CreatedAt)

		line := fmt.Sprintf(" %s  %-40s  %s", icon, truncate(title, 40), date)

		// Only show project if filtering by project (we know the name)
		if m.project != "" {
			line += dimStyle.Render(fmt.Sprintf("  [%s]", m.project))
		}

		if i == m.cursor {
			b.WriteString(selectedStyle.Width(m.width).Render("▶" + line))
		} else {
			b.WriteString(normalStyle.Width(m.width).Render(" " + line))
		}
		b.WriteString("\n")
	}

	// Pad remaining lines
	rendered := end - m.offset
	for i := rendered; i < h; i++ {
		b.WriteString("\n")
	}

	// Status bar
	count := len(m.notes)
	status := fmt.Sprintf(" %d notes  │  j/k navigate  │  enter open  │  / search  │  d archive  │  r refresh  │  q quit", count)
	b.WriteString(statusBarStyle.Width(m.width).Render(status))

	return b.String()
}

func typeIcon(t string) string {
	switch t {
	case "voice":
		return "🎙"
	case "link":
		return "🔗"
	default:
		return "📝"
	}
}

func noteTitle(n Note) string {
	if n.Title != nil && *n.Title != "" {
		return *n.Title
	}
	if n.Content != nil && *n.Content != "" {
		line := strings.SplitN(*n.Content, "\n", 2)[0]
		return strings.TrimPrefix(line, "# ")
	}
	return "(untitled)"
}

func formatDate(d *string) string {
	if d == nil {
		return "          "
	}
	if len(*d) >= 10 {
		return (*d)[:10]
	}
	return *d
}

func truncate(s string, max int) string {
	runes := []rune(s)
	if len(runes) <= max {
		return s
	}
	return string(runes[:max-1]) + "…"
}
