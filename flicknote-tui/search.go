package main

import (
	"fmt"
	"strings"
)

func (m Model) viewSearch() string {
	var b strings.Builder

	// Title bar
	titleBar := titleStyle.Width(m.width).Render(" FlickNote — Search")
	b.WriteString(titleBar)
	b.WriteString("\n")

	// Search input
	inputBox := searchBoxStyle.Width(m.width - 4).Render(m.searchInput.View())
	b.WriteString(inputBox)
	b.WriteString("\n")

	if m.err != nil {
		b.WriteString(errorStyle.Render(fmt.Sprintf(" Error: %v", m.err)))
	}
	b.WriteString("\n")

	// Dimmed note list behind search
	listHeight := m.height - 6
	if listHeight < 0 {
		listHeight = 0
	}

	end := listHeight
	if end > len(m.notes) {
		end = len(m.notes)
	}

	for i := 0; i < end; i++ {
		title := noteTitle(m.notes[i])
		line := fmt.Sprintf("   %s", truncate(title, 50))
		b.WriteString(dimStyle.Render(line))
		b.WriteString("\n")
	}

	// Pad
	for i := end; i < listHeight; i++ {
		b.WriteString("\n")
	}

	// Status bar
	status := " type to search  │  enter search  │  esc cancel"
	b.WriteString(statusBarStyle.Width(m.width).Render(status))

	return b.String()
}
