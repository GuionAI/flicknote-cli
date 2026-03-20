package main

import (
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"
)

type viewState int

const (
	stateList viewState = iota
	stateDetail
	stateSearch
)

type Model struct {
	state   viewState
	client  *Client
	project string // --project filter (empty = all)

	// List view
	notes  []Note
	cursor int
	offset int

	// Detail view
	detail        *NoteDetail
	detailMD      string   // glamour-rendered markdown
	detailContent []string // pre-split lines of full detail (metadata + content)
	scrollOffset  int

	// Search
	searchInput textinput.Model
	searchQuery string

	// Layout
	width  int
	height int

	// Status
	statusMsg string
	err       error
}

func NewModel(project string) Model {
	ti := textinput.New()
	ti.Placeholder = "search keywords..."
	ti.CharLimit = 256

	return Model{
		state:       stateList,
		client:      &Client{},
		project:     project,
		searchInput: ti,
	}
}

// --- Messages ---

type notesLoadedMsg struct{ notes []Note }
type noteDetailMsg struct {
	detail   *NoteDetail
	rendered string
	lines    []string // pre-split content lines
}
type archiveDoneMsg struct{}
type errMsg struct{ err error }

// --- Init ---

func (m Model) Init() tea.Cmd {
	return m.loadNotes()
}

func (m Model) loadNotes() tea.Cmd {
	return func() tea.Msg {
		var notes []Note
		var err error
		if m.searchQuery != "" {
			keywords := strings.Fields(m.searchQuery)
			notes, err = m.client.SearchNotes(keywords...)
		} else if m.project != "" {
			notes, err = m.client.ListNotesForProject(m.project)
		} else {
			notes, err = m.client.ListNotes()
		}
		if err != nil {
			return errMsg{err}
		}
		return notesLoadedMsg{notes}
	}
}

// --- View (returns tea.View, NOT string) ---

func (m Model) View() tea.View {
	var content string
	if m.width == 0 {
		content = "loading..."
	} else {
		switch m.state {
		case stateList:
			content = m.viewList()
		case stateDetail:
			content = m.viewDetail()
		case stateSearch:
			content = m.viewSearch()
		}
	}
	v := tea.NewView(content)
	v.AltScreen = true
	return v
}

// --- Update ---

func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		return m, nil

	case notesLoadedMsg:
		m.notes = msg.notes
		m.cursor = 0
		m.offset = 0
		m.err = nil
		return m, nil

	case noteDetailMsg:
		m.detail = msg.detail
		m.detailMD = msg.rendered
		m.detailContent = msg.lines
		m.scrollOffset = 0
		m.err = nil
		m.state = stateDetail
		return m, nil

	case archiveDoneMsg:
		m.err = nil
		return m, m.loadNotes()

	case errMsg:
		m.err = msg.err
		return m, nil

	// IMPORTANT: Use tea.KeyPressMsg, NOT tea.KeyMsg.
	// tea.KeyMsg matches both press AND release — using it causes double-fire.
	case tea.KeyPressMsg:
		return m.handleKey(msg)
	}

	if m.state == stateSearch {
		var cmd tea.Cmd
		m.searchInput, cmd = m.searchInput.Update(msg)
		return m, cmd
	}

	return m, nil
}

func (m Model) handleKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if msg.String() == "ctrl+c" {
		return m, tea.Quit
	}

	switch m.state {
	case stateList:
		return m.handleListKey(msg)
	case stateDetail:
		return m.handleDetailKey(msg)
	case stateSearch:
		return m.handleSearchKey(msg)
	}
	return m, nil
}

// listHeight returns the visible height for the note list.
func (m Model) listHeight() int {
	h := m.height - 3 // title + status + padding
	if h < 1 {
		return 1
	}
	return h
}

// adjustListOffset ensures the cursor is visible within the list viewport.
// Call this in Update() after any cursor movement.
func (m *Model) adjustListOffset() {
	h := m.listHeight()
	if m.cursor < m.offset {
		m.offset = m.cursor
	}
	if m.cursor >= m.offset+h {
		m.offset = m.cursor - h + 1
	}
}

func (m Model) handleListKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "q":
		return m, tea.Quit
	case "j", "down":
		if m.cursor < len(m.notes)-1 {
			m.cursor++
		}
		m.adjustListOffset()
	case "k", "up":
		if m.cursor > 0 {
			m.cursor--
		}
		m.adjustListOffset()
	case "g", "home":
		m.cursor = 0
		m.adjustListOffset()
	case "G", "end":
		if len(m.notes) > 0 {
			m.cursor = len(m.notes) - 1
		}
		m.adjustListOffset()
	case "enter", "l", "right":
		if len(m.notes) > 0 {
			return m, m.loadDetail(m.notes[m.cursor].ID)
		}
	case "/":
		m.searchInput.SetValue(m.searchQuery)
		m.searchInput.Focus()
		m.state = stateSearch
		return m, nil
	case "d":
		if len(m.notes) > 0 {
			id := m.notes[m.cursor].ID
			return m, func() tea.Msg {
				if err := m.client.ArchiveNote(id); err != nil {
					return errMsg{err}
				}
				return archiveDoneMsg{}
			}
		}
	case "r":
		return m, m.loadNotes()
	}
	return m, nil
}

// detailMaxScroll returns the maximum scroll offset for the detail view.
func (m Model) detailMaxScroll() int {
	visible := m.height - 3
	max := len(m.detailContent) - visible
	if max < 0 {
		return 0
	}
	return max
}

// clampDetailScroll ensures scrollOffset is within valid bounds.
// Call this in Update() after any scroll change.
func (m *Model) clampDetailScroll() {
	max := m.detailMaxScroll()
	if m.scrollOffset > max {
		m.scrollOffset = max
	}
	if m.scrollOffset < 0 {
		m.scrollOffset = 0
	}
}

func (m Model) handleDetailKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "q", "esc", "h", "left":
		m.state = stateList
		m.detail = nil
	case "j", "down":
		m.scrollOffset++
		m.clampDetailScroll()
	case "k", "up":
		if m.scrollOffset > 0 {
			m.scrollOffset--
		}
	case "ctrl+d":
		m.scrollOffset += m.height / 2
		m.clampDetailScroll()
	case "ctrl+u":
		m.scrollOffset -= m.height / 2
		m.clampDetailScroll()
	case "g", "home":
		m.scrollOffset = 0
	case "G", "end":
		m.scrollOffset = m.detailMaxScroll()
	}
	return m, nil
}

func (m Model) handleSearchKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "esc":
		m.state = stateList
		m.searchInput.Blur()
		return m, nil
	case "enter":
		m.searchQuery = m.searchInput.Value()
		m.state = stateList
		m.searchInput.Blur()
		return m, m.loadNotes()
	}

	var cmd tea.Cmd
	m.searchInput, cmd = m.searchInput.Update(msg)
	return m, cmd
}

func (m Model) loadDetail(id string) tea.Cmd {
	width := m.width
	return func() tea.Msg {
		detail, err := m.client.GetNote(id)
		if err != nil {
			return errMsg{err}
		}
		rendered := ""
		if detail.Content != nil {
			rendered = renderMarkdown(*detail.Content, width-4)
		}
		// Pre-split lines so View() can render without rebuilding on every frame
		fullContent := buildDetailContent(detail, rendered)
		lines := strings.Split(fullContent, "\n")
		return noteDetailMsg{detail, rendered, lines}
	}
}
