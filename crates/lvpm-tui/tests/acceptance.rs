//! The TUI's scenarios: cucumber-rs drives the real `App` on a ratatui
//! `TestBackend`, pressing keys and reading the drawn screen.
//!
//! `harness = false`: run it with `cargo test -p lvpm-tui --test acceptance`.
//! A feature file can hold scenarios for both: this harness runs those with a
//! step on the TUI, the root crate's `tests/workspace.rs` the others. A
//! skipped step fails the run.

mod steps;

use cucumber::World;
use cucumber::gherkin::Scenario;
use cucumber::writer::Stats;
use lvpm_tui::app::App;
use lvpm_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::path::PathBuf;

/// A scenario with a step that starts with this is one of the TUI's.
const TUI_STEP_PREFIX: &str = "the TUI ";

/// The features, relative to the workspace root. Later slices add theirs here.
const FEATURE_FILES: &[&str] =
    &["tests/features/lvpm-tui/slice-4-the-lvpm-tui-crate-starts-and-quits.feature"];

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct TuiWorld {
    pub app: App,
    pub terminal: Terminal<TestBackend>,
}

impl TuiWorld {
    fn new() -> TuiWorld {
        TuiWorld {
            app: App::new(),
            terminal: Terminal::new(TestBackend::new(ui::MIN_WIDTH, ui::MIN_HEIGHT))
                .expect("a test terminal"),
        }
    }
}

fn is_tui_scenario(scenario: &Scenario) -> bool {
    scenario
        .steps
        .iter()
        .any(|step| step.value.starts_with(TUI_STEP_PREFIX))
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let failures: Vec<String> = FEATURE_FILES
        .iter()
        .filter_map(|feature| {
            let writer = futures::executor::block_on(
                TuiWorld::cucumber()
                    // `cargo test <filter>` hands every test binary libtest
                    // arguments, which cucumber's own CLI rejects.
                    .with_default_cli()
                    .fail_on_skipped()
                    .filter_run(root.join(feature), |_, _, scenario| {
                        is_tui_scenario(scenario)
                    }),
            );
            // A filter or prefix that drifted would otherwise pass by running nothing.
            if writer.passed_steps() == 0 {
                return Some(format!("{feature}: no TUI scenario ran"));
            }
            writer.execution_has_failed().then(|| {
                format!(
                    "{feature}: {} step(s) failed, {} parsing error(s)",
                    writer.failed_steps(),
                    writer.parsing_errors()
                )
            })
        })
        .collect();
    if !failures.is_empty() {
        panic!("{}", failures.join("\n"));
    }
}
