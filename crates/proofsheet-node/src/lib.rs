//! Node.js binding for proofsheet.
//!
//! The capture loop is blocking and launches a browser per device, so it must
//! never run on the JavaScript thread. Each call is dispatched to libuv's
//! thread pool through `napi::Task`, which is why the exported functions
//! return promises.
//!
//! Nothing here reimplements the run loop. It marshals arguments into
//! `proofsheet_core::RunOptions`, calls the same `run` the CLI calls, and
//! marshals the report back. A binding that grows its own logic drifts from
//! the CLI, and then "works on the command line" becomes a support burden.

use napi::bindgen_prelude::*;
use napi::{Env, Task};
use napi_derive::napi;

use proofsheet_core::{cdp, device, Collector, Determinism, RunOptions, Stability};

/// Options accepted by `capture`.
#[napi(object)]
#[derive(Clone)]
pub struct CaptureOptions {
    /// Page to capture.
    pub url: String,
    /// Where PNGs are written. Defaults to `./proofsheet-out`.
    pub out_dir: Option<String>,
    /// Explicit device ids. Takes precedence over `store`.
    pub devices: Option<Vec<String>>,
    /// Capture every preset for `apple`, `play` or `web`.
    pub store: Option<String>,
    /// Determinism seed. Defaults to 42.
    pub seed: Option<i64>,
    /// Locale override, e.g. `en-US`.
    pub locale: Option<String>,
    /// Timezone override, e.g. `UTC`.
    pub timezone: Option<String>,
    /// Path to a Chromium binary. Defaults to discovery.
    pub browser: Option<String>,
    /// Stop at the first failure.
    pub fail_fast: Option<bool>,
    /// Path to a preset JSON file, replacing the built-in table.
    pub presets: Option<String>,
}

fn parse_store(s: &str) -> Option<device::Store> {
    match s.to_ascii_lowercase().as_str() {
        "apple" | "ios" | "appstore" => Some(device::Store::Apple),
        "play" | "android" | "google" => Some(device::Store::Play),
        "web" => Some(device::Store::Web),
        _ => None,
    }
}

fn load_table(presets: Option<&String>) -> Result<Vec<device::Device>> {
    match presets {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| Error::from_reason(format!("cannot read {path}: {e}")))?;
            device::parse_presets(&text).map_err(|e| Error::from_reason(e.to_string()))
        }
        None => Ok(device::builtin()),
    }
}

/// Turn the JS options into core options, failing loudly on anything unknown.
fn build_options(o: &CaptureOptions) -> Result<RunOptions> {
    let table = load_table(o.presets.as_ref())?;

    let devices = if let Some(ids) = &o.devices {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            // An unknown id is an error, never a skip. Silently capturing a
            // smaller set than requested yields an incomplete upload that
            // looks like it worked.
            let d = device::by_id(&table, id)
                .ok_or_else(|| Error::from_reason(format!("unknown device id: {id}")))?;
            out.push(d);
        }
        out
    } else if let Some(s) = &o.store {
        let store =
            parse_store(s).ok_or_else(|| Error::from_reason(format!("unknown store: {s}")))?;
        let out = device::for_store(&table, store);
        if out.is_empty() {
            return Err(Error::from_reason(format!("no presets for store {s}")));
        }
        out
    } else {
        return Err(Error::from_reason(
            "specify either `devices` or `store`".to_string(),
        ));
    };

    let mut det = Determinism::default();
    if let Some(seed) = o.seed {
        if seed < 0 {
            return Err(Error::from_reason("seed must not be negative".to_string()));
        }
        det.seed = seed as u64;
    }
    if let Some(l) = &o.locale {
        det.locale = l.clone();
    }
    if let Some(t) = &o.timezone {
        det.timezone = t.clone();
    }

    let browser = match &o.browser {
        Some(p) => std::path::PathBuf::from(p),
        None => cdp::find_browser(None).map_err(|e| Error::from_reason(e.to_string()))?,
    };

    Ok(RunOptions {
        url: o.url.clone(),
        devices,
        out_dir: std::path::PathBuf::from(
            o.out_dir
                .clone()
                .unwrap_or_else(|| "./proofsheet-out".into()),
        ),
        determinism: det,
        stability: Stability::default(),
        browser,
        fail_fast: o.fail_fast.unwrap_or(false),
    })
}

/// What the page reported about itself during a capture.
#[napi(object)]
pub struct JsEnvironment {
    pub inner_width: u32,
    pub inner_height: u32,
    pub device_pixel_ratio: u32,
    pub touch_points: u32,
    /// False means the page overrode the layout viewport, which usually means
    /// content negotiation served the wrong layout at the right pixel count.
    pub viewport_honoured: bool,
}

/// One image, as JavaScript sees it.
#[napi(object)]
pub struct JsCapture {
    /// Pixel width the store requires.
    pub expected_width: u32,
    pub expected_height: u32,
    /// Pixel width actually produced.
    pub actual_width: u32,
    pub actual_height: u32,
    /// Content address of the PNG bytes.
    pub sha256: String,
    pub bytes: u32,
    /// True when actual matches expected exactly.
    pub exact: bool,
    /// What the page saw. Assert on this as well as the dimensions: a capture
    /// can be exactly the right size and still show the wrong layout.
    pub environment: Option<JsEnvironment>,
}

/// One device's result, successful or not.
#[napi(object)]
pub struct JsDeviceResult {
    pub device_id: String,
    /// `exact`, `off-size` or `failed`.
    pub outcome: String,
    pub elapsed_ms: u32,
    pub capture: Option<JsCapture>,
    /// Present only when the capture failed.
    pub error: Option<String>,
    /// Where the PNG was written.
    pub path: Option<String>,
}

/// The result of a capture run.
#[napi(object)]
pub struct ProofsheetReport {
    /// Version of the native core that produced this.
    pub proofsheet: String,
    pub url: String,
    pub seed: i64,
    pub locale: String,
    pub exact: u32,
    pub off_size: u32,
    pub failed: u32,
    pub elapsed_ms: u32,
    /// True only when at least one capture happened and none went wrong.
    /// Callers should branch on this rather than on `failed === 0`, which is
    /// also true for a run that captured nothing at all.
    pub ok: bool,
    pub results: Vec<JsDeviceResult>,
}

impl From<proofsheet_core::RunReport> for ProofsheetReport {
    fn from(r: proofsheet_core::RunReport) -> Self {
        ProofsheetReport {
            proofsheet: r.proofsheet,
            url: r.url,
            seed: r.seed as i64,
            locale: r.locale,
            exact: r.exact as u32,
            off_size: r.off_size as u32,
            failed: r.failed as u32,
            elapsed_ms: r.elapsed_ms as u32,
            ok: r.ok,
            results: r
                .results
                .into_iter()
                .map(|d| JsDeviceResult {
                    device_id: d.device_id,
                    outcome: d.outcome,
                    elapsed_ms: d.elapsed_ms as u32,
                    capture: d.capture.map(|c| JsCapture {
                        expected_width: c.expected.0,
                        expected_height: c.expected.1,
                        actual_width: c.actual.0,
                        actual_height: c.actual.1,
                        sha256: c.sha256,
                        bytes: c.bytes as u32,
                        exact: c.exact,
                        environment: c.environment.map(|e| JsEnvironment {
                            inner_width: e.inner_width,
                            inner_height: e.inner_height,
                            device_pixel_ratio: e.device_pixel_ratio,
                            touch_points: e.touch_points,
                            viewport_honoured: e.viewport_honoured,
                        }),
                    }),
                    error: d.error,
                    path: d.path,
                })
                .collect(),
        }
    }
}

/// Runs the blocking capture on the libuv pool.
pub struct CaptureTask {
    options: CaptureOptions,
}

impl Task for CaptureTask {
    /// `Output` crosses the thread boundary so it must be `Send`; JS values
    /// are bound to the JS thread, so the conversion happens in `resolve`.
    type Output = proofsheet_core::RunReport;
    type JsValue = ProofsheetReport;

    fn compute(&mut self) -> Result<Self::Output> {
        let opts = build_options(&self.options)?;
        let mut collector = Collector::default();
        proofsheet_core::run(&opts, &mut collector).map_err(|e| Error::from_reason(e.to_string()))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output.into())
    }
}

/// Capture screenshots at exact store dimensions.
///
/// Resolves with a [`ProofsheetReport`]. Branch on `report.ok`, not on
/// `report.failed === 0` — the latter is also true when nothing was captured.
#[napi]
pub fn capture(options: CaptureOptions) -> AsyncTask<CaptureTask> {
    AsyncTask::new(CaptureTask { options })
}

/// List device presets, optionally filtered to one store.
#[napi]
pub fn devices(store: Option<String>, presets: Option<String>) -> Result<serde_json::Value> {
    let table = load_table(presets.as_ref())?;
    let filtered: Vec<&device::Device> = match store {
        Some(s) => {
            let st =
                parse_store(&s).ok_or_else(|| Error::from_reason(format!("unknown store: {s}")))?;
            table.iter().filter(|d| d.store == st).collect()
        }
        None => table.iter().collect(),
    };
    serde_json::to_value(&filtered)
        .map_err(|e| Error::from_reason(format!("serialize devices: {e}")))
}

/// Locate the browser proofsheet would use, or throw explaining why it cannot.
#[napi]
pub fn find_browser() -> Result<String> {
    cdp::find_browser(None)
        .map(|p| p.display().to_string())
        .map_err(|e| Error::from_reason(e.to_string()))
}

/// The native core version. Kept in lockstep with the npm package version by
/// a CI check, because a binding reporting a different version than the
/// package it ships in makes every bug report ambiguous.
#[napi]
pub fn version() -> String {
    proofsheet_core::VERSION.to_string()
}
