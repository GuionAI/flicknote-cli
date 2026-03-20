package main

import (
	"flag"
	"fmt"
	"os"

	tea "charm.land/bubbletea/v2"
)

func main() {
	project := flag.String("project", "", "filter notes by project name")
	flag.Parse()

	m := NewModel(*project)
	// No tea.WithAltScreen() — in bubbletea v2, alt screen is set via
	// view.AltScreen = true in the View() method.
	p := tea.NewProgram(m)
	if _, err := p.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}
