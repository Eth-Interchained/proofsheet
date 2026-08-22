//! Progress reporting.
//!
//! A capture run over a full store matrix launches a browser per device and
//! takes real seconds each. Silence for a minute is indistinguishable from a
//! hang, so the core emits events and the caller decides how to render them.
//!
//! The core deliberately does **not** print. A library that writes to stdout
//! is unusable from a binding, and this crate is about to be consumed from
//! Node and Python where the host owns the console.

use std::fmt;
use std::time::Duration;

use crate::capture::Capture;
use crate::device::Device;

/// What happened to one device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Captured at exactly the required size.
    Exact,
    /// Captured, but the pixels do not match the requirement.
    OffSize,
    /// Did not capture at all.
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Exact => "exact",
            Outcome::OffSize => "off-size",
            Outcome::Failed => "failed",
        }
    }

    /// Whether this outcome should make the overall run fail.
    pub fn is_bad(self) -> bool {
        !matches!(self, Outcome::Exact)
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A tally of a finished run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub exact: usize,
    pub off_size: usize,
    pub failed: usize,
}

impl Summary {
    pub fn total(&self) -> usize {
        self.exact + self.off_size + self.failed
    }

    /// A run is successful only if something happened and nothing went wrong.
    ///
    /// The `total() > 0` clause is deliberate: a run that captured nothing
    /// has an empty problem list, and without this it would report success.
    /// An empty green is the most misleading result a tool can produce.
    pub fn ok(&self) -> bool {
        self.total() > 0 && self.off_size == 0 && self.failed == 0
    }

    fn record(&mut self, o: Outcome) {
        match o {
            Outcome::Exact => self.exact += 1,
            Outcome::OffSize => self.off_size += 1,
            Outcome::Failed => self.failed += 1,
        }
    }
}

/// Everything known about one finished device.
///
/// Passed as a struct rather than eight positional parameters, so adding a
/// field later does not break every implementor — and so nobody transposes
/// two same-typed arguments at a call site.
#[derive(Debug, Clone, Copy)]
pub struct DeviceEvent<'a> {
    /// 1-based position in the run.
    pub index: usize,
    pub total: usize,
    pub device: &'a Device,
    pub outcome: Outcome,
    /// Present unless the capture failed outright.
    pub capture: Option<&'a Capture>,
    pub elapsed: Duration,
    /// Present only on failure.
    pub error: Option<&'a str>,
}

/// Events emitted as a run proceeds.
///
/// Every method has a default no-op body so an implementor can observe only
/// what it cares about.
pub trait Progress {
    /// Called once, before any device is touched.
    fn run_started(&mut self, _total: usize, _url: &str) {}

    /// Called before each device, with a 1-based index.
    fn device_started(&mut self, _index: usize, _total: usize, _device: &Device) {}

    /// Called after each device, successful or not.
    fn device_finished(&mut self, _event: &DeviceEvent<'_>) {}

    /// Called once, after every device.
    fn run_finished(&mut self, _summary: Summary, _elapsed: Duration) {}
}

/// Discards everything. Useful for tests and for callers that want silence.
#[derive(Debug, Default, Clone, Copy)]
pub struct Silent;

impl Progress for Silent {}

/// Collects outcomes in memory. Used by the bindings, which return a result
/// object rather than streaming to a console.
#[derive(Debug, Default, Clone)]
pub struct Collector {
    pub summary: Summary,
    pub events: Vec<(String, Outcome, u128)>,
}

impl Progress for Collector {
    fn device_finished(&mut self, e: &DeviceEvent<'_>) {
        self.summary.record(e.outcome);
        self.events
            .push((e.device.id.clone(), e.outcome, e.elapsed.as_millis()));
    }
}

/// Tally outcomes without holding onto them.
#[derive(Debug, Default, Clone, Copy)]
pub struct Tally(pub Summary);

impl Progress for Tally {
    fn device_finished(&mut self, e: &DeviceEvent<'_>) {
        self.0.record(e.outcome);
    }
}

/// Render a `current/total` bar of the given width.
///
/// Pure and separately testable, because off-by-one bars are the classic
/// place a progress indicator lies about how far along it is.
pub fn bar(current: usize, total: usize, width: usize) -> String {
    if total == 0 || width == 0 {
        return String::new();
    }
    let filled = (current.min(total) * width) / total;
    let mut s = String::with_capacity(width);
    for i in 0..width {
        s.push(if i < filled { '#' } else { '.' });
    }
    s
}

/// Format a duration the way a person reads it.
pub fn human(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", d.as_secs_f64())
    } else {
        format!("{}m{:02}s", d.as_secs() / 60, d.as_secs() % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_run_is_not_a_success() {
        // The whole point: no captures must never read as "all good".
        assert!(!Summary::default().ok());
    }

    #[test]
    fn clean_run_is_a_success() {
        let s = Summary {
            exact: 3,
            ..Default::default()
        };
        assert!(s.ok());
        assert_eq!(s.total(), 3);
    }

    #[test]
    fn any_problem_fails_the_run() {
        assert!(!Summary {
            exact: 5,
            off_size: 1,
            failed: 0
        }
        .ok());
        assert!(!Summary {
            exact: 5,
            off_size: 0,
            failed: 1
        }
        .ok());
    }

    #[test]
    fn bar_endpoints_are_exact() {
        assert_eq!(bar(0, 10, 10), "..........");
        assert_eq!(bar(10, 10, 10), "##########");
        assert_eq!(bar(5, 10, 10), "#####.....");
    }

    #[test]
    fn bar_never_overflows_its_width() {
        for cur in 0..30 {
            assert_eq!(bar(cur, 10, 8).chars().count(), 8);
        }
        // Overshoot must clamp, not panic or run long.
        assert_eq!(bar(99, 10, 8), "########");
    }

    #[test]
    fn bar_handles_degenerate_input() {
        assert_eq!(bar(1, 0, 10), "");
        assert_eq!(bar(1, 10, 0), "");
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(human(Duration::from_millis(250)), "250ms");
        assert_eq!(human(Duration::from_millis(1500)), "1.5s");
        assert_eq!(human(Duration::from_secs(125)), "2m05s");
    }

    #[test]
    fn outcome_badness() {
        assert!(!Outcome::Exact.is_bad());
        assert!(Outcome::OffSize.is_bad());
        assert!(Outcome::Failed.is_bad());
    }

    #[test]
    fn collector_records_every_device() {
        let d = crate::device::builtin().into_iter().next().unwrap();
        let mut c = Collector::default();
        for outcome in [Outcome::Exact, Outcome::Failed] {
            c.device_finished(&DeviceEvent {
                index: 1,
                total: 2,
                device: &d,
                outcome,
                capture: None,
                elapsed: Duration::from_millis(5),
                error: None,
            });
        }
        assert_eq!(c.summary.total(), 2);
        assert_eq!(c.summary.failed, 1);
        assert_eq!(c.events.len(), 2);
        assert!(!c.summary.ok());
    }
}
