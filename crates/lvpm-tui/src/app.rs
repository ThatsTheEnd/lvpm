//! The TUI's state and its event loop.

use crate::{keys, ui};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{Event, KeyEventKind};

#[derive(Debug)]
pub struct App {
    running: bool,
}

impl Default for App {
    fn default() -> App {
        App { running: true }
    }
}

impl App {
    pub fn new() -> App {
        App::default()
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Act on one terminal event. Only key presses count: Windows also
    /// reports each key's release. A resize needs nothing here, as each draw
    /// takes the terminal's size as it is then.
    pub fn handle(&mut self, event: Event) {
        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
            && keys::QUIT.matches(key)
        {
            self.running = false;
        }
    }
}

/// Draw, then wait for and handle the next event, until the app stops running.
pub fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    mut next_event: impl FnMut() -> Result<Event, B::Error>,
) -> Result<(), B::Error> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if !app.is_running() {
            return Ok(());
        }
        app.handle(next_event()?);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn after(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> bool {
        let mut app = App::new();
        app.handle(Event::Key(KeyEvent::new_with_kind(code, modifiers, kind)));
        app.is_running()
    }

    #[test]
    fn ctrl_c_quits() {
        assert!(!after(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press
        ));
    }

    #[test]
    fn q_with_a_modifier_and_a_repeated_q_do_not_quit() {
        assert!(after(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press
        ));
        assert!(after(
            KeyCode::Char('Q'),
            KeyModifiers::SHIFT,
            KeyEventKind::Press
        ));
        assert!(after(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat
        ));
    }
}
