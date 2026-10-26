//! Drawing: the header bar, the body's frame and the key footer, or only a
//! message when the terminal is too small. Reverse video, not colour, marks
//! the bars, so they read the same in conhost and with `NO_COLOR`.

use crate::app::App;
use crate::keys::{self, Binding};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;

pub fn draw(frame: &mut Frame, _app: &App) {
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
    let title = Line::from(vec![
        Span::styled(" lvpm ", reversed().add_modifier(Modifier::BOLD)),
        Span::raw(concat!(" lvpm-tui ", env!("CARGO_PKG_VERSION"))),
    ]);
    frame.render_widget(Paragraph::new(title), header);
    frame.render_widget(Block::bordered(), body);
    frame.render_widget(Paragraph::new(hints(keys::FOOTER)), footer);
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
