//! `--json`: JSON Lines on stdout. Each event is one line, `{"event": name,
//! "data": {...}}`, and the last line is `{"result": ...}` or `{"error":
//! message}`, so a reader tells them apart by their one top-level key.

use lvpm_core::events::{Event, EventSink};

/// Writes each event as one line on stdout.
pub struct JsonSink;

impl EventSink for JsonSink {
    fn emit(&self, event: Event) {
        // `println!` locks stdout for the whole line, so lines from a worker
        // thread do not interleave.
        println!("{}", serde_json::json!(event));
    }
}

/// The line a command that succeeded ends with.
pub fn result(result: &serde_json::Value) -> String {
    serde_json::json!({ "result": result }).to_string()
}

/// The line a command that failed ends with: the error and its causes.
pub fn error(error: &anyhow::Error) -> String {
    serde_json::json!({ "error": format!("{error:#}") }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lvpm_core::events::Phase;

    #[test]
    fn an_event_keeps_its_fields_under_data() {
        let line = serde_json::json!(Event::PhaseStarted {
            phase: Phase::Download
        });
        assert_eq!(
            line,
            serde_json::json!({"event": "phase_started", "data": {"phase": "download"}})
        );
    }

    #[test]
    fn the_last_line_has_only_a_result_or_an_error() {
        let done = result(&serde_json::json!({"command": "install"}));
        assert_eq!(done, r#"{"result":{"command":"install"}}"#);
        let failed = error(&anyhow::anyhow!("no alpha").context("installing"));
        assert_eq!(failed, r#"{"error":"installing: no alpha"}"#);
    }
}
