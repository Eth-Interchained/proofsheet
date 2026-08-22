//! Driving a whole capture matrix.
//!
//! This exists so the CLI, the Node binding and the Python binding all walk
//! the same code path. A binding that reimplements the loop is a binding that
//! drifts from the CLI, and then "it works from the command line" becomes a
//! support burden.

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::capture::{capture, Capture, CaptureRequest, Stability};
use crate::cdp::{Browser, LaunchOptions};
use crate::determinism::Determinism;
use crate::device::Device;
use crate::error::Result;
use crate::progress::{DeviceEvent, Outcome, Progress, Summary};

/// Everything a run needs.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Page to capture.
    pub url: String,
    /// Devices to capture it on.
    pub devices: Vec<Device>,
    /// Where PNGs are written.
    pub out_dir: PathBuf,
    pub determinism: Determinism,
    pub stability: Stability,
    /// Browser binary.
    pub browser: PathBuf,
    /// Stop at the first failure rather than completing the matrix.
    pub fail_fast: bool,
}

/// One device's result, including the failure case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceResult {
    pub device_id: String,
    pub outcome: String,
    pub elapsed_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture: Option<Capture>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The manifest a run produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub proofsheet: String,
    pub url: String,
    pub seed: u64,
    pub locale: String,
    pub exact: usize,
    pub off_size: usize,
    pub failed: usize,
    pub elapsed_ms: u128,
    /// True only when at least one capture happened and none went wrong.
    pub ok: bool,
    pub results: Vec<DeviceResult>,
}

/// Capture every device in `opts`, reporting progress as it goes.
///
/// A device that fails does not abort the matrix unless `fail_fast` is set:
/// when eight of nine sizes are fine, you want the eight and a clear note
/// about the ninth, not an empty output directory.
pub fn run<P: Progress>(opts: &RunOptions, progress: &mut P) -> Result<RunReport> {
    let started = Instant::now();
    let total = opts.devices.len();
    progress.run_started(total, &opts.url);
    std::fs::create_dir_all(&opts.out_dir)?;

    let mut summary = Summary::default();
    let mut results = Vec::with_capacity(total);

    for (i, device) in opts.devices.iter().enumerate() {
        let index = i + 1;
        progress.device_started(index, total, device);
        let t0 = Instant::now();

        let attempt = capture_one(opts, device, &opts.out_dir);
        let elapsed = t0.elapsed();

        let (outcome, cap, err, path) = match attempt {
            Ok((c, p)) => {
                let o = if c.exact {
                    Outcome::Exact
                } else {
                    Outcome::OffSize
                };
                (o, Some(c), None, Some(p))
            }
            Err(e) => (Outcome::Failed, None, Some(e.to_string()), None),
        };

        match outcome {
            Outcome::Exact => summary.exact += 1,
            Outcome::OffSize => summary.off_size += 1,
            Outcome::Failed => summary.failed += 1,
        }

        progress.device_finished(&DeviceEvent {
            index,
            total,
            device,
            outcome,
            capture: cap.as_ref(),
            elapsed,
            error: err.as_deref(),
        });

        results.push(DeviceResult {
            device_id: device.id.clone(),
            outcome: outcome.as_str().to_string(),
            elapsed_ms: elapsed.as_millis(),
            capture: cap,
            error: err,
            path: path.map(|p| p.display().to_string()),
        });

        if opts.fail_fast && outcome.is_bad() {
            break;
        }
    }

    let elapsed = started.elapsed();
    progress.run_finished(summary, elapsed);

    Ok(RunReport {
        proofsheet: crate::VERSION.to_string(),
        url: opts.url.clone(),
        seed: opts.determinism.seed,
        locale: opts.determinism.locale.clone(),
        exact: summary.exact,
        off_size: summary.off_size,
        failed: summary.failed,
        elapsed_ms: elapsed.as_millis(),
        ok: summary.ok(),
        results,
    })
}

/// One device, in its own browser.
///
/// A fresh process per device is deliberate. Emulation overrides accumulate
/// on a session, and a leaked override from the previous device produces a
/// plausible-looking image at the wrong metrics — the exact failure mode this
/// tool exists to prevent. Process startup is cheap next to being wrong.
fn capture_one(opts: &RunOptions, device: &Device, out_dir: &Path) -> Result<(Capture, PathBuf)> {
    let launch = LaunchOptions::new(&opts.browser);
    let mut browser = Browser::launch(&launch)?;
    let req = CaptureRequest {
        url: &opts.url,
        device,
        determinism: &opts.determinism,
        stability: &opts.stability,
    };
    let (cap, bytes) = capture(&mut browser, &req)?;
    let path = out_dir.join(format!("{}.png", device.id));
    std::fs::write(&path, &bytes)?;
    Ok((cap, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::Collector;

    fn opts(devices: Vec<Device>) -> RunOptions {
        RunOptions {
            url: "about:blank".into(),
            devices,
            out_dir: std::env::temp_dir().join("proofsheet-run-test"),
            determinism: Determinism::default(),
            stability: Stability::default(),
            // Deliberately not a browser. These tests exercise the loop's
            // bookkeeping and its failure path, not the browser.
            browser: PathBuf::from("/nonexistent/proofsheet/no-browser-here"),
            fail_fast: false,
        }
    }

    #[test]
    fn every_device_is_reported_even_when_all_fail() {
        let devices: Vec<Device> = crate::device::builtin().into_iter().take(3).collect();
        let mut c = Collector::default();
        let report = run(&opts(devices), &mut c).unwrap();

        assert_eq!(report.results.len(), 3);
        assert_eq!(report.failed, 3);
        assert_eq!(c.summary.total(), 3);
        // A run where nothing captured must never claim success.
        assert!(!report.ok);
        for r in &report.results {
            assert_eq!(r.outcome, "failed");
            assert!(r.error.is_some(), "a failure must carry its reason");
        }
    }

    #[test]
    fn fail_fast_stops_at_the_first_problem() {
        let devices: Vec<Device> = crate::device::builtin().into_iter().take(4).collect();
        let mut o = opts(devices);
        o.fail_fast = true;
        let mut c = Collector::default();
        let report = run(&o, &mut c).unwrap();
        assert_eq!(report.results.len(), 1, "should have stopped after one");
    }

    #[test]
    fn an_empty_device_list_is_not_a_success() {
        let mut c = Collector::default();
        let report = run(&opts(vec![]), &mut c).unwrap();
        assert_eq!(report.results.len(), 0);
        assert!(
            !report.ok,
            "a run that captured nothing reported ok; that is the empty-green bug"
        );
    }
}
