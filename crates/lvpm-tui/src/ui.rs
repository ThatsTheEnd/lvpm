//! Drawing: the header bar, the body and the key footer, or only a message
//! when the terminal is too small. Reverse video, not colour, marks the bars
//! and the selected row, so they read the same in conhost and with `NO_COLOR`.

use crate::app::{App, Listing};
use crate::browse::Browse;
use crate::destination;
use crate::keys::{self, Binding};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Row, Table, Wrap};

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let message = format!(
            "The terminal is {}x{}; lvpm-tui needs at least {MIN_WIDTH}x{MIN_HEIGHT}.",
            area.width, area.height
        );
        frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: true }), area);
        return;
    }
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    let place = match &app.destination {
        Ok(destination) => destination::label(destination),
        Err(_) => "no destination selected".to_string(),
    };
    let title = Line::from(vec![
        Span::styled(" lvpm ", reversed().add_modifier(Modifier::BOLD)),
        Span::raw(format!(" {place}")),
    ]);
    frame.render_widget(Paragraph::new(title), header);

    // What a key could not do, else why nothing can be installed.
    let note = app
        .note
        .clone()
        .or_else(|| app.destination.as_ref().err().cloned());
    let block = Block::bordered();
    let inner = block.inner(body);
    frame.render_widget(block, body);
    match &mut app.listing {
        Listing::Loading => frame.render_widget(Paragraph::new("Loading the catalog…"), inner),
        Listing::Failed(error) => frame.render_widget(
            Paragraph::new(error.as_str()).wrap(Wrap { trim: false }),
            inner,
        ),
        Listing::Loaded(browse) => draw_list(frame, browse, inner),
    }
    // The hints come first so that a long note cannot push them off.
    let mut line = hints(keys::FOOTER);
    if let Some(note) = note.as_deref().and_then(|n| n.lines().next()) {
        line.spans.push(Span::raw(note.to_string()));
    }
    frame.render_widget(Paragraph::new(line), footer);
}

/// The search and filter line, a banner per source not read, and the table.
fn draw_list(frame: &mut Frame, browse: &mut Browse, area: Rect) {
    let banners = browse.failed.len() as u16;
    let [status, banner, list] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(banners),
        Constraint::Min(0),
    ])
    .areas(area);
    let cursor = if browse.searching { "_" } else { "" };
    let line = format!(
        "Search: {}{cursor}   Filter [{}]   {} of {}",
        browse.search,
        browse.filter.label(),
        browse.rows.len(),
        browse.packages.len()
    );
    frame.render_widget(Paragraph::new(line), status);
    let not_read: Vec<Line> = browse
        .failed
        .iter()
        .map(|(source, _)| Line::styled(format!("not read: {source}"), reversed()))
        .collect();
    frame.render_widget(Paragraph::new(not_read), banner);

    if browse.rows.is_empty() {
        let message = if browse.packages.is_empty() {
            format!(
                "The catalog is empty. Sources read:\n{}",
                browse.sources.join("\n")
            )
        } else if browse.search.is_empty() {
            format!("No package is {}.", browse.filter.label())
        } else {
            format!("No package matches \"{}\".", browse.search)
        };
        frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: false }), list);
        return;
    }
    // The header row takes one line; the rest is what PgUp and PgDn move by.
    browse.page = list.height.saturating_sub(1).into();
    let rows = browse.rows.iter().map(|&i| {
        let package = &browse.packages[i];
        Row::new([
            package.name.clone(),
            package
                .installed
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            package
                .available
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            package.status.text(),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Fill(1),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(14),
        ],
    )
    .header(Row::new(["Package", "Installed", "Available", "Status"]).style(reversed()))
    .row_highlight_style(reversed())
    .highlight_symbol("> ");
    frame.render_stateful_widget(table, list, &mut browse.table);
}

fn reversed() -> Style {
    Style::new().add_modifier(Modifier::REVERSED)
}

/// Each key in reverse video, followed by what it does.
fn hints(bindings: &[&Binding]) -> Line<'static> {
    let spans = bindings.iter().flat_map(|binding| {
        [
            Span::styled(binding.key, reversed()),
            Span::raw(format!(" {}  ", binding.action)),
        ]
    });
    Line::from(spans.collect::<Vec<_>>())
}
