//! Python binding for proofsheet.
//!
//! This layer is deliberately thin: it validates arguments, calls the same
//! `proofsheet_core::run` the CLI and the Node binding call, and hands back
//! JSON. The Pythonic surface — dataclasses, keyword arguments, docstrings —
//! lives in `python/proofsheet/__init__.py`, where it is far easier to write
//! and to read than it would be in PyO3.
//!
//! Two things matter here and nowhere else:
//!
//! 1. **The GIL is released** around the capture loop. A run launches a
//!    browser per device and takes real seconds; holding the GIL would freeze
//!    every other thread in the host process for the duration.
//! 2. **Errors carry their reason.** A binding that raises a bare exception
//!    turns a diagnosable failure into a mystery.

use proofsheet_core::{cdp, device, Collector, Determinism, RunOptions, Stability};
use pyo3::exceptions::{PyFileNotFoundError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;

fn parse_store(s: &str) -> Option<device::Store> {
    match s.to_ascii_lowercase().as_str() {
        "apple" | "ios" | "appstore" => Some(device::Store::Apple),
        "play" | "android" | "google" => Some(device::Store::Play),
        "web" => Some(device::Store::Web),
        _ => None,
    }
}

fn load_table(presets: Option<&str>) -> PyResult<Vec<device::Device>> {
    match presets {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| {
                PyFileNotFoundError::new_err(format!("cannot read presets {path}: {e}"))
            })?;
            device::parse_presets(&text)
                .map_err(|e| PyValueError::new_err(format!("invalid presets: {e}")))
        }
        None => Ok(device::builtin()),
    }
}

/// Device presets as a JSON string, optionally filtered to one store.
#[pyfunction]
#[pyo3(signature = (store=None, presets=None))]
fn devices_json(store: Option<&str>, presets: Option<&str>) -> PyResult<String> {
    let table = load_table(presets)?;
    let filtered: Vec<&device::Device> = match store {
        Some(s) => {
            let st = parse_store(s)
                .ok_or_else(|| PyValueError::new_err(format!("unknown store: {s}")))?;
            table.iter().filter(|d| d.store == st).collect()
        }
        None => table.iter().collect(),
    };
    serde_json::to_string(&filtered)
        .map_err(|e| PyRuntimeError::new_err(format!("serialize devices: {e}")))
}

/// Run a capture matrix and return the report as a JSON string.
#[pyfunction]
#[pyo3(signature = (
    url,
    out_dir,
    device_ids=None,
    store=None,
    seed=42,
    locale=None,
    timezone=None,
    browser=None,
    fail_fast=false,
    presets=None,
))]
#[allow(clippy::too_many_arguments)]
fn capture_json(
    py: Python<'_>,
    url: String,
    out_dir: String,
    device_ids: Option<Vec<String>>,
    store: Option<&str>,
    seed: u64,
    locale: Option<String>,
    timezone: Option<String>,
    browser: Option<String>,
    fail_fast: bool,
    presets: Option<&str>,
) -> PyResult<String> {
    let table = load_table(presets)?;

    let selected = if let Some(ids) = device_ids {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            // Unknown ids are an error, never a silent skip: capturing fewer
            // images than asked for produces an incomplete upload that looks
            // like it worked.
            let d = device::by_id(&table, &id)
                .ok_or_else(|| PyValueError::new_err(format!("unknown device id: {id}")))?;
            out.push(d);
        }
        out
    } else if let Some(s) = store {
        let st =
            parse_store(s).ok_or_else(|| PyValueError::new_err(format!("unknown store: {s}")))?;
        let out = device::for_store(&table, st);
        if out.is_empty() {
            return Err(PyValueError::new_err(format!("no presets for store {s}")));
        }
        out
    } else {
        return Err(PyValueError::new_err("specify either device_ids or store"));
    };

    let mut det = Determinism {
        seed,
        ..Default::default()
    };
    if let Some(l) = locale {
        det.locale = l;
    }
    if let Some(t) = timezone {
        det.timezone = t;
    }

    let browser_path = match browser {
        Some(p) => std::path::PathBuf::from(p),
        None => cdp::find_browser(None).map_err(|e| PyRuntimeError::new_err(e.to_string()))?,
    };

    let opts = RunOptions {
        url,
        devices: selected,
        out_dir: std::path::PathBuf::from(out_dir),
        determinism: det,
        stability: Stability::default(),
        browser: browser_path,
        fail_fast,
    };

    // Release the GIL: this blocks for seconds per device.
    let report = py.allow_threads(|| {
        let mut collector = Collector::default();
        proofsheet_core::run(&opts, &mut collector)
    });
    let report = report.map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

    serde_json::to_string(&report)
        .map_err(|e| PyRuntimeError::new_err(format!("serialize report: {e}")))
}

/// Path to the browser proofsheet would use.
#[pyfunction]
fn find_browser() -> PyResult<String> {
    cdp::find_browser(None)
        .map(|p| p.display().to_string())
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

/// Version of the native core.
#[pyfunction]
fn version() -> &'static str {
    proofsheet_core::VERSION
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(devices_json, m)?)?;
    m.add_function(wrap_pyfunction!(capture_json, m)?)?;
    m.add_function(wrap_pyfunction!(find_browser, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add("__version__", proofsheet_core::VERSION)?;
    Ok(())
}
