# proofsheet-core

The engine behind [proofsheet](https://crates.io/crates/proofsheet): exact-pixel
App Store and Google Play screenshots from a real browser, deterministic and
local-first.

This crate is the library. For the command-line tool, install
[`proofsheet`](https://crates.io/crates/proofsheet).

```toml
[dependencies]
proofsheet-core = "0.1"
```

```rust
use proofsheet_core::{device, find_browser, progress::Silent, run,
                      Determinism, RunOptions, Stability, Store};

let presets = device::builtin();
let report = run(&RunOptions {
    url: "http://localhost:5173".into(),
    devices: device::for_store(&presets, Store::Apple),
    out_dir: "./shots".into(),
    determinism: Determinism { seed: 42, ..Default::default() },
    stability: Stability::default(),
    browser: find_browser(None)?,
    fail_fast: false,
}, &mut Silent)?;

// `ok`, not `failed == 0` -- the latter is also true when nothing was captured.
assert!(report.ok, "{} off-size, {} failed", report.off_size, report.failed);
```

## What it does that a screenshot script doesn't

**Exact pixels by construction.** A preset stores the output size the store
requires and derives the viewport as `output / scale`. A preset whose output
doesn't divide evenly is rejected at parse time, so "the size we emit is a size
the store accepts" is structural rather than arithmetic someone got right.

**Deterministic runs.** A preamble injected before any page script seeds
`Math.random`, freezes `Date.now` and `performance.now`, drives
`requestAnimationFrame` on a fixed virtual step, and pins locale and timezone
through CDP. Same seed, byte-identical PNGs.

**Wrong-layout detection.** A page can be captured at exactly the right pixel
count and still show a desktop layout scaled into a phone frame. Every
`Capture` carries an `Environment` recording what the page actually did —
assert on `viewport_honoured`, not just the dimensions.

No async runtime, no browser automation framework, no HTTP client. The
WebSocket and CDP client are hand-rolled `std`.

## Documentation

Full docs, device tables and examples:
[github.com/interchained/proofsheet](https://github.com/interchained/proofsheet)

## License

BUSL-1.1, converting to MIT on 2030-08-22.
Property made in part by **Interchained LLC Labs**.
