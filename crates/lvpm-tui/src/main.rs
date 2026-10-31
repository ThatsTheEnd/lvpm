//! `lvpm-tui`: lvpm in a terminal user interface. Released as `lvpm tui`;
//! this binary is for working on the TUI alone.

use clap::Parser;

#[derive(Parser)]
#[command(version, about)]
struct Cli {}

fn main() -> anyhow::Result<()> {
    Cli::parse();
    lvpm_tui::run()
}
