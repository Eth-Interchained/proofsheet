# Examples

The same program in all three languages: capture a site at **every** App
Store and Google Play size, write each store to its own folder, exit non-zero
if anything is off-size or failed.

Every example here was run against a live site before being committed. They
are checked in CI, so they cannot silently rot.

```
44 exact, 0 problem(s)
  apple/  33 PNGs
  play/   11 PNGs
```

| | run it |
|---|---|
| [Node](node) | `npm i && node shots.js https://example.com ./shots` |
| [Python](python) | `pip install proofsheet && python shots.py https://example.com ./shots` |
| [Rust](rust) | `cargo run --release -- https://example.com ./shots` |

Any URL works, including a dev server — `http://localhost:3000` is a normal
target.

## The one thing worth copying

Checking the output size is not enough:

```js
!capture.environment.viewportHonoured   // Node
not capture.environment.viewport_honoured   # Python
!capture.environment.viewport_honoured  // Rust
```

A page can be captured at exactly the pixel count a store demands and still
show completely the wrong thing. If the site serves a desktop layout and
declares `<meta name="viewport" content="width=1120">`, Chrome honours that
meta tag, lays the page out at 1120 CSS px, and scales the desktop design
down into a phone-sized frame. The file is 1320x2868 and passes every
dimension check. It is still a desktop screenshot.

Measured on a real site, changing only the User-Agent and touch points:

| | metrics only | + UA + touch |
|---|---|---|
| `innerWidth` | 1120 | 440 |
| `maxTouchPoints` | 0 | 5 |
| meta viewport | `width=1120` | `width=device-width` |

So assert on the environment, not just the dimensions. Each example prints a
warning when a page overrides the layout viewport.

## The second thing

Branch on `ok`, never on `failed == 0`:

```python
if report.ok:      # right
if not report.failed:   # wrong -- also true when NOTHING was captured
```

A run that captured zero images has zero failures. `ok` additionally requires
that at least one capture actually happened.

## Determinism

All three pass `seed=42`. The same seed against unchanged content reproduces
byte-identical PNGs, which is what makes a regenerated screenshot set
diffable — you can commit them and review changes as a diff.

## Browser

proofsheet finds Chrome/Chromium automatically. To pin one:

```
export PROOFSHEET_CHROME=/path/to/chrome
```

Empty means unset, so `export PROOFSHEET_CHROME=$(which chrome)` falls back to
discovery on a machine without it rather than failing.
