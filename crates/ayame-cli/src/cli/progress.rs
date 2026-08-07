use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use std::collections::HashSet;

/// One line of the serve→worker progress protocol, which
/// `serve::ops::parse_progress_line` reads back off the worker's stderr.
/// `done` is clamped so a worker that overcounts cannot report above 100%.
pub(crate) fn machine_progress_line(done: u64, total: u64) -> String {
    format!("ayame-progress\t{}\t{}", done.min(total), total)
}

pub(crate) struct ProgressReporter {
    label: &'static str,
    mode: ProgressMode,
    state: Mutex<ProgressState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgressMode {
    Off,
    Human,
    Machine,
}

struct ProgressState {
    last: Instant,
    wrote_human: bool,
}

impl ProgressReporter {
    pub(crate) fn new(label: &'static str, flags: &HashSet<String>) -> ProgressReporter {
        let mode = if flags.contains("--progress") {
            ProgressMode::Machine
        } else if std::io::stderr().is_terminal() {
            ProgressMode::Human
        } else {
            ProgressMode::Off
        };
        ProgressReporter {
            label,
            mode,
            state: Mutex::new(ProgressState {
                last: Instant::now()
                    .checked_sub(Duration::from_secs(1))
                    .unwrap_or_else(Instant::now),
                wrote_human: false,
            }),
        }
    }

    pub(crate) fn report(&self, done: u64, total: u64) {
        if self.mode == ProgressMode::Off {
            return;
        }
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        if done < total && now.duration_since(state.last) < Duration::from_millis(250) {
            return;
        }
        state.last = now;
        match self.mode {
            ProgressMode::Machine => {
                eprintln!("{}", machine_progress_line(done, total));
            }
            ProgressMode::Human => {
                let pct = if total == 0 {
                    100.0
                } else {
                    (done.min(total) as f64 / total as f64) * 100.0
                };
                if self.label == "sort" {
                    // Sort progress is normalized across scanning and every
                    // merge pass, so its units are work rather than lines.
                    eprint!("\r{}: {pct:.1}%", self.label);
                } else {
                    eprint!(
                        "\r{}: {} / {} lines ({pct:.1}%)",
                        self.label,
                        crate::commas(done.min(total)),
                        crate::commas(total)
                    );
                }
                let _ = std::io::stderr().flush();
                state.wrote_human = true;
            }
            ProgressMode::Off => {}
        }
    }

    pub(crate) fn finish(&self) {
        if self.mode != ProgressMode::Human {
            return;
        }
        let mut state = self.state.lock().unwrap();
        if state.wrote_human {
            eprintln!();
            state.wrote_human = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// `--progress` is the serve→worker contract: the supervisor parses these
    /// exact lines off the worker's stderr to drive the progress card, so the
    /// mode selection and the line format are an API, not formatting (#113).
    #[test]
    fn the_progress_flag_selects_the_machine_protocol() {
        assert_eq!(
            ProgressReporter::new("sort", &flags(&["--progress"])).mode,
            ProgressMode::Machine
        );
    }

    /// Without the flag, a worker whose stderr is a pipe — which is how the
    /// server spawns it — must stay silent rather than write human progress
    /// into a stream nobody renders.
    #[test]
    fn a_piped_worker_without_the_flag_is_silent() {
        // Tests run with stderr captured (not a terminal), which is the same
        // shape as the server's pipe.
        assert_eq!(
            ProgressReporter::new("sort", &flags(&[])).mode,
            ProgressMode::Off
        );
    }

    #[test]
    fn machine_progress_lines_match_what_the_supervisor_parses() {
        assert_eq!(machine_progress_line(12, 100), "ayame-progress\t12\t100");
        // A worker that counts past the total still reports a value the
        // supervisor can turn into a percentage.
        assert_eq!(machine_progress_line(120, 100), "ayame-progress\t100\t100");
    }
}
