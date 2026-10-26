//! The TUI's state and its event loop.

use crate::browse::{Browse, Status};
use crate::destination::EnvBuilder;
use crate::worker::{self, Done, Job};
use crate::{keys, ui};
use lvpm_core::ops::Destination;
use lvpm_core::target::LvTarget;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::sync::mpsc::{Receiver, TryRecvError};

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
    /// The running job's result, when it comes. At most one job runs.
    pending: Option<Receiver<Done>>,
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
            pending: None,
        };
        app.load(false);
        app
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    fn target(&self) -> Option<&LvTarget> {
        self.destination.as_ref().ok().and_then(Destination::target)
    }

    fn load(&mut self, refresh: bool) {
        self.listing = Listing::Loading;
        self.pending = Some(worker::spawn(&self.builder, Job::Load { refresh }));
    }

    fn finish(&mut self, done: Done) {
        self.pending = None;
        match done {
            Done::Loaded {
                destination,
                listing,
            } => {
                // The installed list was read from this destination, so the
                // header and the LabVIEW gate follow it.
                self.destination = destination;
                self.listing = match listing {
                    Ok((catalog, installed)) => {
                        Listing::Loaded(Browse::new(catalog, &installed, self.target()))
                    }
                    Err(error) => Listing::Failed(error),
                }
            }
        }
    }

    /// The worker ended without a result: it panicked.
    fn lost(&mut self) {
        self.pending = None;
        self.listing = Listing::Failed("reading the catalog stopped without a result".into());
    }

    /// Take the running job's result if it has come.
    pub fn poll(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
        match pending.try_recv() {
            Ok(done) => self.finish(done),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.lost(),
        }
    }

    /// Block until the running job's result has come, if one is running.
    pub fn wait_for_job(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
        match pending.recv() {
            Ok(done) => self.finish(done),
            Err(_) => self.lost(),
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
            match self.pending {
                Some(_) => self.note = Some("the catalog is still loading".into()),
                None => self.load(true),
            }
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
        KeyCode::Char(c) if TYPING.contains(&key.modifiers) => browse.type_char(c),
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

/// The modifiers a typed character can come with. Windows reports AltGr,
/// which types `@` or `\` on some layouts, as Ctrl+Alt.
const TYPING: [KeyModifiers; 3] = [
    KeyModifiers::NONE,
    KeyModifiers::SHIFT,
    KeyModifiers::CONTROL.union(KeyModifiers::ALT),
];

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
            pending: None,
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

    fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        app.handle(Event::Key(KeyEvent::new(code, modifiers)));
    }

    /// An idle app showing `alpha`, which fits, and `gamma`, which needs
    /// LabVIEW 2099.
    fn listing() -> App {
        let entry = |name: &str, lv_min: f64| lvpm_core::index::Entry {
            name: name.into(),
            version: lvpm_core::version::Version::parse("1.0"),
            url: String::new(),
            md5: None,
            display_name: None,
            requires: Vec::new(),
            lv_min: Some(lv_min),
            source: String::new(),
        };
        let catalog = lvpm_core::ops::query::Catalog {
            entries: vec![entry("alpha", 20.0), entry("gamma", 99.0)],
            sources: Vec::new(),
            failed: Vec::new(),
        };
        let labview_2025 = LvTarget {
            version: 25.0,
            bitness: 64,
            path: "LabVIEW 2025".into(),
        };
        let mut app = idle();
        app.listing = Listing::Loaded(Browse::new(catalog, &[], Some(&labview_2025)));
        app
    }

    fn browse(app: &App) -> &Browse {
        match &app.listing {
            Listing::Loaded(browse) => browse,
            other => panic!("not loaded: {other:?}"),
        }
    }

    #[test]
    fn the_search_box_takes_q_as_a_letter_and_still_quits_on_ctrl_c() {
        let mut app = listing();
        press(&mut app, KeyCode::Char('/'), KeyModifiers::SHIFT);
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('x'), KeyModifiers::CONTROL);
        press(
            &mut app,
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert!(app.is_running());
        assert_eq!(
            browse(&app).search,
            "q@",
            "Ctrl-x types nothing, AltGr does"
        );
        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(browse(&app).search, "q");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(!browse(&app).searching);
        assert_eq!(browse(&app).search, "q", "Enter keeps the search");

        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(!app.is_running());
    }

    #[test]
    fn a_note_shows_only_for_a_gated_package_and_until_the_next_key() {
        let mut app = listing();
        press(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
        assert_eq!(app.note, None, "alpha fits");
        press(&mut app, KeyCode::Down, KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
        assert_eq!(app.note.as_deref(), Some("gamma needs LabVIEW 2099"));
        press(&mut app, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(app.note, None);
    }

    #[test]
    fn reload_while_a_job_runs_starts_no_second_one() {
        let mut app = idle();
        let (_send, receive) = std::sync::mpsc::channel();
        app.pending = Some(receive);
        press(&mut app, KeyCode::Char('R'), KeyModifiers::SHIFT);
        assert_eq!(app.note.as_deref(), Some("the catalog is still loading"));
    }

    #[test]
    fn a_worker_that_ends_without_a_result_fails_the_listing() {
        let mut app = idle();
        let (send, receive) = std::sync::mpsc::channel::<Done>();
        app.pending = Some(receive);
        app.poll();
        assert!(matches!(app.listing, Listing::Loading));
        drop(send);
        app.poll();
        assert!(
            matches!(app.listing, Listing::Failed(_)),
            "{:?}",
            app.listing
        );
        assert!(app.pending.is_none());
    }

    #[test]
    fn the_loop_shows_the_result_that_arrives_between_keys() {
        let mut app = idle();
        let (send, receive) = std::sync::mpsc::channel();
        app.pending = Some(receive);
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        let mut ticks = 0;
        run(&mut terminal, &mut app, || {
            ticks += 1;
            Ok(match ticks {
                1 => {
                    send.send(Done::Loaded {
                        destination: Err("none".into()),
                        listing: Err("no source could be read".into()),
                    })
                    .unwrap();
                    None
                }
                2 => None,
                _ => Some(Event::Key(KeyEvent::new(
                    KeyCode::Char('q'),
                    KeyModifiers::NONE,
                ))),
            })
        })
        .unwrap();
        let screen = format!("{:?}", terminal.backend().buffer());
        assert!(screen.contains("no source could be read"), "{screen}");
    }
}
