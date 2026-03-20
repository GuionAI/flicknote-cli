package main

import "charm.land/lipgloss/v2"

var (
	// Colors (matching ttal-cli palette)
	colorDim    = lipgloss.Color("241")
	colorWhite  = lipgloss.Color("255")
	colorBlue   = lipgloss.Color("63")
	colorYellow = lipgloss.Color("220")

	// Styles
	titleStyle = lipgloss.NewStyle().
			Bold(true).
			Foreground(colorWhite).
			Background(colorBlue)

	selectedStyle = lipgloss.NewStyle().
			Bold(true).
			Background(lipgloss.Color("237"))

	normalStyle = lipgloss.NewStyle()

	dimStyle = lipgloss.NewStyle().
			Foreground(colorDim)

	errorStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("196"))

	statusBarStyle = lipgloss.NewStyle().
			Foreground(colorDim)

	searchBoxStyle = lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(colorYellow).
			Padding(0, 1)
)
