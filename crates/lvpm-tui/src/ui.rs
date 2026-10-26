//! Drawing: the header bar, the body and the key footer, or only a message
//! when the terminal is too small. Reverse video, not colour, marks the bars
//! and the selected row, so they read the same in conhost and with `NO_COLOR`.

use crate::app::Details;
use crate::app::{App, Listing};
use crate::browse::Browse;
use crate::destination;
use crate::detail::{self, Detail, Focus};
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
    let mut title = title;
    if app.reloading() {
        title.spans.push(Span::raw("   reloading…"));
    }
    frame.render_widget(Paragraph::new(title), header);

    if let Some(detail) = &mut app.detail {
        draw_detail(frame, detail, &app.details, body);
        let mut line = hints(keys::DETAIL_FOOTER);
        if let Some(note) = &app.note {
            line.spans.insert(0, Span::raw(format!("{note}  ")));
        }
        frame.render_widget(Paragraph::new(line), footer);
        return;
    }
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
    // What a key could not do lasts one key, so it comes first and whole.
    // Why nothing can be installed lasts, so the hints go before it.
    let mut line = hints(keys::FOOTER);
    if let Some(note) = &app.note {
        line.spans.insert(0, Span::raw(format!("{note}  ")));
    } else if let Some(error) = app.destination.as_ref().err() {
        line.spans.push(Span::raw(
            error.lines().next().unwrap_or_default().to_string(),
        ));
    }
    frame.render_widget(Paragraph::new(line), footer);
}

/// The search and filter line, the banners, and the table.
fn draw_list(frame: &mut Frame, browse: &mut Browse, area: Rect) {
    // A failed reload's banner, then one per source not read.
    let mut banners: Vec<Line> = Vec::new();
    if let Some(error) = &browse.stale {
        let why = "Reloading failed; the list shown is from the cache:";
        banners.push(Line::styled(why, reversed()));
        banners.extend(
            error
                .lines()
                .map(|l| Line::styled(l.to_string(), reversed())),
        );
    }
    let not_read = browse.failed.iter();
    banners.extend(
        not_read.map(|(source, _)| Line::styled(format!("not read: {source}"), reversed())),
    );
    let [status, banner, list] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(banners.len() as u16),
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
    frame.render_widget(Paragraph::new(banners), banner);

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

/// The versions on the left, the picked version's fields on the right; the
/// pane Up and Down move has the bold title.
fn draw_detail(frame: &mut Frame, detail: &mut Detail, details: &Details, area: Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Length(32), Constraint::Min(0)]).areas(area);
    let title = |text: String, focused: bool| match focused {
        true => Line::styled(text, Style::new().add_modifier(Modifier::BOLD)),
        false => Line::raw(text),
    };
    let rows = detail.versions.iter().map(|entry| {
        let installed = detail.installed.as_ref() == Some(&entry.version);
        let year = entry.lv_min.map(detail::labview_year);
        Row::new([
            if installed { "*" } else { "" }.to_string(),
            entry.version.to_string(),
            year.map(|y| format!("LV {y}+")).unwrap_or_default(),
        ])
    });
    let versions = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(8),
        ],
    )
    .block(Block::bordered().title(title(" Versions ".into(), detail.focus == Focus::Versions)))
    .row_highlight_style(reversed())
    .highlight_symbol("> ");
    frame.render_stateful_widget(versions, left, &mut detail.table);

    let entry = detail.entry();
    let (name, version) = detail.key();
    let text = detail::fields(detail, entry, details.get(&(name.clone(), version.clone())));
    let block = Block::bordered()
        .title(title(
            format!(" {name} {version} "),
            detail.focus == Focus::Fields,
        ))
        .title_bottom(" PgUp/PgDn scroll ");
    let inner = block.inner(right);
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    // Stop the scroll where the last line reaches the bottom.
    let lines = paragraph.line_count(inner.width) as u16;
    detail.page = inner.height;
    detail.scroll = detail.scroll.min(lines.saturating_sub(inner.height));
    frame.render_widget(paragraph.scroll((detail.scroll, 0)).block(block), right);
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
