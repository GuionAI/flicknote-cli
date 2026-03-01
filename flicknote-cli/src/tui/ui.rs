use super::app::{App, View};
use ratatui::prelude::*;
use ratatui::widgets::*;

fn type_icon(note_type: &str) -> &str {
    match note_type {
        "voice" => "🎙",
        "link" => "🔗",
        _ => "📝",
    }
}

fn format_date(date: Option<&str>) -> &str {
    date.and_then(|d| d.get(..10)).unwrap_or("-")
}

fn project_name<'a>(app: &'a App, project_id: &str) -> Option<&'a str> {
    app.projects
        .iter()
        .find(|p| p.id == project_id)
        .map(|p| p.name.as_str())
}

pub(crate) fn draw(frame: &mut Frame, app: &App) {
    match app.view {
        View::List => draw_list(frame, app),
        View::Detail => draw_detail(frame, app),
        View::Search => draw_search(frame, app),
    }
}

fn draw_list(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    // Title bar
    let title = if app.search_query.is_empty() {
        " FlickNote".to_string()
    } else {
        format!(" FlickNote — search: \"{}\"", app.search_query)
    };
    let title_bar =
        Paragraph::new(title).style(Style::new().bold().fg(Color::White).bg(Color::Blue));
    frame.render_widget(title_bar, chunks[0]);

    // Note list
    let items: Vec<ListItem> = app
        .notes
        .iter()
        .map(|note| {
            let icon = type_icon(&note.r#type);
            let title = note.title.as_deref().unwrap_or("(untitled)");
            let date = format_date(note.created_at.as_deref());
            let truncated_title: String = if title.chars().count() > 40 {
                title.chars().take(40).collect()
            } else {
                title.to_string()
            };
            let mut spans = vec![
                Span::raw(format!(" {icon} ")),
                Span::styled(
                    format!("{truncated_title:<40}"),
                    Style::new().fg(Color::White),
                ),
                Span::raw("  "),
                Span::styled(date, Style::new().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::styled(&note.status, Style::new().fg(Color::Yellow)),
            ];
            if let Some(ref pid) = note.project_id
                && let Some(name) = project_name(app, pid)
            {
                spans.push(Span::styled(
                    format!("  [{name}]"),
                    Style::new().fg(Color::Magenta),
                ));
            }
            let line = Line::from(spans);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::new().bg(Color::DarkGray).bold())
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(list, chunks[1], &mut state);

    // Status bar
    let count = app.notes.len();
    let status = Paragraph::new(format!(
        " {count} notes  │  j/k navigate  │  enter open  │  / search  │  d archive  │  q quit"
    ))
    .style(Style::new().fg(Color::DarkGray));
    frame.render_widget(status, chunks[2]);
}

fn draw_detail(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let Some(note) = app.selected_note() else {
        return;
    };

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    // Title bar
    let icon = type_icon(&note.r#type);
    let title = note.title.as_deref().unwrap_or("(untitled)");
    let title_bar = Paragraph::new(format!(" {icon} {title}"))
        .style(Style::new().bold().fg(Color::White).bg(Color::Blue));
    frame.render_widget(title_bar, chunks[0]);

    // Note detail
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("ID:       ", Style::new().fg(Color::DarkGray)),
            Span::raw(&note.id),
        ]),
        Line::from(vec![
            Span::styled("Type:     ", Style::new().fg(Color::DarkGray)),
            Span::raw(&note.r#type),
        ]),
        Line::from(vec![
            Span::styled("Status:   ", Style::new().fg(Color::DarkGray)),
            Span::styled(&note.status, Style::new().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("Created:  ", Style::new().fg(Color::DarkGray)),
            Span::raw(note.created_at.as_deref().unwrap_or("-")),
        ]),
        Line::from(vec![
            Span::styled("Updated:  ", Style::new().fg(Color::DarkGray)),
            Span::raw(note.updated_at.as_deref().unwrap_or("-")),
        ]),
    ];

    if let Some(ref pid) = note.project_id {
        let name = project_name(app, pid).unwrap_or(pid.as_str());
        lines.push(Line::from(vec![
            Span::styled("Project:  ", Style::new().fg(Color::DarkGray)),
            Span::raw(name),
        ]));
    }

    if let Some(url) = note.link_url() {
        lines.push(Line::from(vec![
            Span::styled("Link:     ", Style::new().fg(Color::DarkGray)),
            Span::styled(url, Style::new().fg(Color::Cyan).underlined()),
        ]));
    }

    if note.is_flagged == Some(1) {
        lines.push(Line::from(vec![
            Span::styled("Flagged:  ", Style::new().fg(Color::DarkGray)),
            Span::styled("yes", Style::new().fg(Color::Red)),
        ]));
    }

    if note.summary.is_some() || note.content.is_some() {
        lines.push(Line::raw(""));
    }

    if let Some(ref summary) = note.summary {
        lines.push(Line::styled(
            "── Summary ──",
            Style::new().fg(Color::DarkGray),
        ));
        for line in summary.lines() {
            lines.push(Line::raw(line));
        }
        lines.push(Line::raw(""));
    }

    if let Some(ref content) = note.content {
        lines.push(Line::styled(
            "── Content ──",
            Style::new().fg(Color::DarkGray),
        ));
        for line in content.lines() {
            lines.push(Line::raw(line));
        }
    }

    let detail = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(Block::default().padding(Padding::horizontal(1)));
    frame.render_widget(detail, chunks[1]);

    // Status bar
    let status = Paragraph::new(" q/esc back to list").style(Style::new().fg(Color::DarkGray));
    frame.render_widget(status, chunks[2]);
}

fn dimmed_note_list(app: &App) -> Vec<ListItem<'_>> {
    app.notes
        .iter()
        .map(|note| {
            let title = note.title.as_deref().unwrap_or("(untitled)");
            ListItem::new(format!("  {title}")).style(Style::new().fg(Color::DarkGray))
        })
        .collect()
}

fn draw_search(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    // Title bar
    let title_bar = Paragraph::new(" FlickNote — Search")
        .style(Style::new().bold().fg(Color::White).bg(Color::Blue));
    frame.render_widget(title_bar, chunks[0]);

    // Search input
    let input = Paragraph::new(app.search_input.as_str()).block(
        Block::bordered()
            .title(" Search ")
            .border_style(Style::new().fg(Color::Yellow)),
    );
    frame.render_widget(input, chunks[1]);

    // Set cursor position
    let cursor_x = chunks[1]
        .x
        .saturating_add(1)
        .saturating_add(app.search_input.chars().count() as u16);
    let cursor_y = chunks[1].y + 1;
    frame.set_cursor_position((cursor_x.min(chunks[1].right() - 2), cursor_y));

    // Autocomplete suggestions or dimmed note list
    let dimmed_notes = dimmed_note_list(app);
    if !app.autocomplete_matches.is_empty() {
        let ac_height = app.autocomplete_matches.len().min(8) as u16;
        let sub =
            Layout::vertical([Constraint::Length(ac_height), Constraint::Min(0)]).split(chunks[2]);

        let suggestions: Vec<ListItem> = app
            .autocomplete_matches
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let style = if i == app.autocomplete_index {
                    Style::new().fg(Color::Yellow).bold()
                } else {
                    Style::new().fg(Color::DarkGray)
                };
                ListItem::new(format!("  {name}")).style(style)
            })
            .collect();
        frame.render_widget(List::new(suggestions), sub[0]);
        frame.render_widget(List::new(dimmed_notes), sub[1]);
    } else {
        frame.render_widget(List::new(dimmed_notes), chunks[2]);
    }

    // Status bar
    let status = Paragraph::new(
        " type to search  │  project:name to filter  │  tab autocomplete  │  enter search  │  esc cancel",
    )
    .style(Style::new().fg(Color::DarkGray));
    frame.render_widget(status, chunks[3]);
}
