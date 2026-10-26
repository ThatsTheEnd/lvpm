//! The TUI's state and its event loop.

use crate::browse::{Browse, Status};
use crate::destination::{self, EnvBuilder};
use crate::worker::{self, Done, Job};
use crate::{keys, ui};
use lvpm_core::ops::Destination;
use lvpm_core::target::LvTarget;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::sync::mpsc::Receiver;

/// The catalog as far as it has been read.
#[derive(Debug)]
pub enum Listing {
    Loading,
    /// Core's error, whole: no source could be read.
    Failed(String),
    Loaded(Browse),
}

#[derive(Debug)]
pub struct App {
    running: bool,
    builder: EnvBuilder,
    /// Where installs go, or why there is nowhere.
    pub destination: Result<Destination, String>,
    pub listing: Listing,
    /// What the last key could not do, shown until the next key.
    pub note: Option<String>,
    job: Option<Receiver<Done>>,
}

impl App {
    /// Resolve the destination and start reading the catalog.
    pub fn new(builder: EnvBuilder) -> App {
        let mut app = App {
            running: true,
            destination: builder.destination(),
            builder,
            listing: Listing::Loading,
            note: None,
            job: None,
        };
        app.load(false);
        app
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    fn target(&self) -> Option<&LvTarget> {
        self.destination.as_ref().ok().and_then(destination::target)
    }

    fn load(&mut self, refresh: bool) {
        self.listing = Listing::Loading;
        self.job = Some(worker::spawn(&self.builder, Job::Load { refresh }));
    }

    fn finish(&mut self, done: Done) {
        self.job = None;
        match done {
            Done::Loaded(loaded) => {
                self.listing = match loaded {
                    Ok((catalog, installed)) => {
                        Listing::Loaded(Browse::new(catalog, &installed, self.target()))
                    }
                    Err(error) => Listing::Failed(error),
                }
            }
        }
    }

    /// Take the running job's result if it has come.
    pub fn poll(&mut self) {
        if let Some(done) = self.job.as_ref().and_then(|job| job.try_recv().ok()) {
            self.finish(done);
        }
    }

    /// Block until the running job's result has come, if one is running.
    pub fn wait_for_job(&mut self) {
        if let Some(done) = self.job.as_ref().and_then(|job| job.recv().ok()) {
            self.finish(done);
        }
    }

    /// Act on one terminal event. Only key presses count: Windows also
    /// reports each key's release. A resize needs nothing here, as each draw
    /// takes the terminal's size as it is then.
    pub fn handle(&mut self, event: Event) {
        let Event::Key(key) = event else {
            return;
        };
        if key.kind != KeyEventKind::Press {
            return;
        }
        self.note = None;
        if let Listing::Loaded(browse) = &mut self.listing
            && browse.searching
        {
            if keys::INTERRUPT.matches(key) {
                self.running = false;
            } else {
                search_key(browse, key);
            }
            return;
        }
        if keys::QUIT.matches(key) {
            self.running = false;
        } else if keys::RELOAD.matches(key) {
            self.load(true);
        } else if let Listing::Loaded(browse) = &mut self.listing {
            self.note = browse_key(browse, key);
        }
    }
}

/// A key while the search box has the keys: Enter keeps the search, Esc
/// drops it.
fn search_key(browse: &mut Browse, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => browse.clear_search(),
        KeyCode::Enter => browse.searching = false,
        KeyCode::Backspace => browse.backspace(),
        KeyCode::Char(c) if matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            browse.type_char(c)
        }
        _ => {}
    }
}

/// A key on the list; what it could not do, if anything.
fn browse_key(browse: &mut Browse, key: KeyEvent) -> Option<String> {
    let page = browse.page_size();
    let moves: [(&keys::Binding, isize); 6] = [
        (&keys::UP, -1),
        (&keys::DOWN, 1),
        (&keys::PAGE_UP, -page),
        (&keys::PAGE_DOWN, page),
        (&keys::HOME, isize::MIN),
        (&keys::END, isize::MAX),
    ];
    if let Some(&(_, by)) = moves.iter().find(|(binding, _)| binding.matches(key)) {
        browse.step(by);
    } else if keys::SEARCH.matches(key) {
        browse.searching = true;
    } else if keys::FILTER.matches(key) {
        browse.next_filter();
    } else if keys::INSTALL.matches(key)
        && let Some(package) = browse.selected()
        && let Status::NeedsLabview(year) = package.status
    {
        // shortcut: install of a compatible package does nothing until the
        // plan preview exists (Slice 6).
        return Some(format!("{} needs LabVIEW {year}", package.name));
    }
    None
}

/// Draw, take the worker's result, then handle the next event if one came,
/// until the app stops running.
pub fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    mut next_event: impl FnMut() -> Result<Option<Event>, B::Error>,
) -> Result<(), B::Error> {
    loop {
        app.poll();
        terminal.draw(|frame| ui::draw(frame, app))?;
        if !app.is_running() {
            return Ok(());
        }
        if let Some(event) = next_event()? {
            app.handle(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An app with nothing loading, so no test reads a feed.
    fn idle() -> App {
        let builder = EnvBuilder::new(
            lvpm_core::config::Config {
                cache: "cache".into(),
                local_sources: Vec::new(),
                venv_dir: ".venv".into(),
            },
            Vec::new(),
            ".".into(),
        );
        App {
            running: true,
            destination: builder.destination(),
            builder,
            listing: Listing::Loading,
            note: None,
            job: None,
        }
    }

    fn after(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> bool {
        let mut app = idle();
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
