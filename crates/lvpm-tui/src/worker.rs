//! Runs core's operations off the UI thread, one job at a time, and hands
//! each result back over a channel the event loop polls.

use crate::destination::{EnvBuilder, Quiet};
use lvpm_core::install::Manifest;
use lvpm_core::ops::Destination;
use lvpm_core::ops::query::{self, Catalog};
use std::sync::mpsc::{self, Receiver};

/// Every job the worker can run.
#[derive(Debug, Clone, Copy)]
pub enum Job {
    /// Read the catalog and what the destination has installed; `refresh`
    /// revalidates the feeds as `--refresh` does.
    Load { refresh: bool },
}

/// What a job returns.
#[derive(Debug)]
pub enum Done {
    /// The destination as resolved for this read, and the catalog with what
    /// that destination has installed, or core's error, whole.
    Loaded {
        destination: Result<Destination, String>,
        listing: Result<(Catalog, Vec<Manifest>), String>,
    },
}

/// Start `job` on its own thread; its result arrives on the receiver.
pub fn spawn(builder: &EnvBuilder, job: Job) -> Receiver<Done> {
    let (send, receive) = mpsc::channel();
    let builder = builder.clone();
    std::thread::spawn(move || {
        // The TUI has gone when nobody receives; the result is not needed then.
        send.send(run(&builder, job)).ok();
    });
    receive
}

fn run(builder: &EnvBuilder, job: Job) -> Done {
    match job {
        Job::Load { refresh } => {
            let env = builder.env(&Quiet, refresh);
            let destination = builder.destination();
            let listing = query::catalog(&env, None).and_then(|catalog| {
                let installed = match destination.is_ok() {
                    true => query::list(&env)?,
                    false => Vec::new(),
                };
                Ok((catalog, installed))
            });
            Done::Loaded {
                destination,
                listing: listing.map_err(|error| format!("{error:#}")),
            }
        }
    }
}
