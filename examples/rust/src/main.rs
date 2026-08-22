//! Capture a site at every App Store and Google Play size.
//!
//!     cargo run -- https://ourlynx.com ./shots
//!
//! Exits non-zero if anything is off-size or failed, so it drops into CI
//! without extra plumbing.

use std::path::PathBuf;
use std::process::ExitCode;

use proofsheet_core::{
    device, find_browser, progress::Silent, run, Determinism, RunOptions, Stability, Store,
    VERSION,
};

const STORES: [(&str, Store); 2] = [("apple", Store::Apple), ("play", Store::Play)];

fn main() -> ExitCode {
    let mut argv = std::env::args().skip(1);
    let url = argv.next().unwrap_or_else(|| "https://ourlynx.com".into());
    let out_root = PathBuf::from(argv.next().unwrap_or_else(|| "./shots".into()));

    let browser = match find_browser(None) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("no browser: {e}");
            return ExitCode::FAILURE;
        }
    };

    // VERSION is the core's, not this example's -- printing
    // env!("CARGO_PKG_VERSION") here reports the example crate and is a lie.
    println!("proofsheet {VERSION}");
    println!("browser    {}", browser.display());
    println!("url        {url}");
    println!("out        {}\n", out_root.display());

    let presets = device::builtin();
    let mut total_exact = 0usize;
    let mut total_bad = 0usize;

    for (name, store) in STORES {
        let devices = device::for_store(&presets, store);
        let opts = RunOptions {
            url: url.clone(),
            devices,
            out_dir: out_root.join(name),
            // The same seed reproduces byte-identical images, which is what
            // makes a regenerated screenshot set diffable.
            determinism: Determinism { seed: 42, ..Default::default() },
            stability: Stability::default(),
            browser: browser.clone(),
            fail_fast: false,
        };

        print!("{name:<6} capturing... ");
        let report = match run(&opts, &mut Silent) {
            Ok(r) => r,
            Err(e) => {
                println!("failed: {e}");
                total_bad += 1;
                continue;
            }
        };

        println!(
            "{} exact, {} off-size, {} failed  ({:.1}s)",
            report.exact,
            report.off_size,
            report.failed,
            report.elapsed_ms as f64 / 1000.0
        );

        // A shot can be exactly the right number of pixels and still show the
        // wrong layout: if the page overrides the layout viewport you get the
        // desktop skin scaled into a phone frame. Size alone will not catch it.
        let wrong: Vec<_> = report
            .results
            .iter()
            .filter_map(|r| r.capture.as_ref().and_then(|c| c.environment.as_ref()).map(|e| (r, e)))
            .filter(|(_, e)| !e.viewport_honoured)
            .collect();
        if !wrong.is_empty() {
            println!("       WARNING: {} shot(s) got the wrong layout", wrong.len());
            for (r, e) in wrong.iter().take(3) {
                println!("         {}: laid out at {}px CSS", r.device_id, e.inner_width);
            }
        }

        for r in report.results.iter().filter(|r| r.outcome != "exact") {
            match &r.capture {
                Some(c) => println!(
                    "       {} {}: wanted {}x{}, got {}x{}",
                    r.outcome.to_uppercase(),
                    r.device_id,
                    c.expected.0,
                    c.expected.1,
                    c.actual.0,
                    c.actual.1
                ),
                None => println!(
                    "       {} {}: {}",
                    r.outcome.to_uppercase(),
                    r.device_id,
                    r.error.as_deref().unwrap_or("unknown")
                ),
            }
        }

        total_exact += report.exact;
        total_bad += report.off_size + report.failed;
        // report.ok, not failed == 0 -- the latter is also true for a run
        // that captured nothing at all.
        if !report.ok {
            total_bad = total_bad.max(1);
        }
    }

    println!("\n{total_exact} exact, {total_bad} problem(s) -> {}", out_root.display());
    if total_bad == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
