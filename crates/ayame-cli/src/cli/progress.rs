use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use std::collections::HashSet;

pub(crate) struct ProgressReporter {
    label: &'static str,
    mode: ProgressMode,
    state: Mutex<ProgressState>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
                eprintln!("ayame-progress\t{}\t{}", done.min(total), total);
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
