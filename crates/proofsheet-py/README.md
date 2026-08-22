# proofsheet

**Exact-pixel App Store and Google Play screenshots from a real browser. Deterministic, local-first, no cloud.**

**Source:** [github.com/interchained/proofsheet](https://github.com/interchained/proofsheet)

```bash
pip install proofsheet
```

```python
import proofsheet

report = proofsheet.capture(
    "https://your.app",
    out_dir="./shots",
    store="apple",
)

print(report.summary())
if not report.ok:
    raise SystemExit(1)
```

```
33 exact, 0 off-size, 0 failed in 12043ms
```

---

## Point it at anything a browser can open

There is no "local mode" and no "remote mode" — the first argument is a URL. All four of these are ordinary usage:

```python
proofsheet.capture("http://localhost:5173", store="apple", out_dir="./shots")   # dev server
proofsheet.capture(f"file://{os.getcwd()}/dist/index.html", store="apple")      # static build
proofsheet.capture("https://pr-482.preview.example.com", store="apple")         # preview deploy
proofsheet.capture("https://your.app", store="apple")                           # production
```

**Prefer localhost.** Not as a fallback — as the default:

- **You capture before you ship.** The set is built from the branch you're about to release, so it can *gate* the release rather than document it afterwards.
- **CI needs no deploy and no public URL.** Start your dev server, capture, upload.
- **It's hermetic.** A live domain drags in CDN state, cookie banners, A/B buckets and analytics — all of which move between runs and destroy byte-identical determinism. Localhost doesn't.
- **It's faster.** Measured on the same page: `366ms` local vs `1.6s` over the network. Across a 44-device matrix that's ~25 seconds versus a couple of minutes.
- **It works on apps that aren't public yet**, or are behind auth.

A complete pre-release script:

```python
import contextlib
import socket
import subprocess
import time

import proofsheet


@contextlib.contextmanager
def dev_server(cmd, port, timeout=30):
    proc = subprocess.Popen(cmd, shell=True)
    try:
        # Wait for the port to accept connections rather than sleeping a
        # hopeful number of seconds.
        deadline = time.time() + timeout
        while time.time() < deadline:
            with socket.socket() as s:
                s.settimeout(0.5)
                if s.connect_ex(("127.0.0.1", port)) == 0:
                    break
            time.sleep(0.25)
        else:
            raise TimeoutError(f"nothing listening on {port} after {timeout}s")
        yield f"http://localhost:{port}"
    finally:
        proc.terminate()
        proc.wait(timeout=10)


with dev_server("npm run preview", 4173) as url:
    report = proofsheet.capture(url, store="apple", out_dir="./shots")

print(report.summary())
raise SystemExit(0 if report.ok else 1)
```

Two things worth knowing before they bite you:

**`file://` is not a real origin.** Fastest path, fine for genuinely static pages, but `fetch`, service workers, ES module imports and anything CORS-sensitive behave differently there than in production. If your app does real work, run the dev server and point at `localhost`.

**Inside Docker, `localhost` means the container.** If this runs in a container while your dev server runs on the host, use `host.docker.internal` or `--network host`.

## Why

Store screenshot sets rot. They get taken by hand, at different moments, on different builds — one shot says 3:47 and the next says 9:12, a list reshuffles between frames, "2 hours ago" becomes "3 days ago". Then a redesign lands and somebody spends two days redoing all of them.

proofsheet makes the set a build artifact. Same seed, same bytes. Rerun after a redesign and the only things that changed are the things you changed.

## Exact pixels, structurally

Apple and Google publish requirements in **output pixels** — `1320 × 2868`, `1024 × 500`. A browser is driven in **CSS pixels** plus a device pixel ratio. Storing the CSS size and multiplying is the obvious design and it's the wrong one: it permits a preset that cannot produce a required size, and you find out at upload.

So a preset stores the required output size and *derives* the viewport as `output / scale`. Any preset where that division isn't exact is rejected before a browser starts.

```python
d = proofsheet.devices("apple")[0]
d.output_size   # (1320, 2868)  <- what the store requires
d.viewport      # (440, 956)    <- what the browser is driven at
d.scale         # 3
d.source        # the Apple doc URL this came from
```

## Determinism

A preamble is injected before any page script on every document: seeded PRNG, frozen clock, virtual `requestAnimationFrame`, seeded `crypto.getRandomValues`. Locale and timezone are pinned through CDP rather than script, because the script-level overrides don't reach Intl's internal data.

Verified in both directions — the same seed produces byte-identical PNGs across independent browser launches, and a different seed produces different bytes. The second half is what makes the first half evidence.

```python
a = proofsheet.capture(url, device_ids=["apple-iphone-6-9-1320"], seed=42)
b = proofsheet.capture(url, device_ids=["apple-iphone-6-9-1320"], seed=42)
assert a.results[0].capture.sha256 == b.results[0].capture.sha256
```

## Presets

46 presets, every store size read from official documentation on 2026-08-22 and carrying the URL it came from.

- **Apple** — iPhone 6.9″/6.5″/6.3″/6.1″/5.5″/4.7″, iPad 13″/11″/10.5″/9.7″, Mac, Apple TV, Vision Pro, Apple Watch
- **Google Play** — feature graphic, phone, 7″/10″ tablet, Wear OS, Automotive, TV

```python
for d in proofsheet.devices("play"):
    if d.mandatory:
        print(d.id, d.output_size)
```

Pass `presets="my-devices.json"` to use your own table. Stores change these numbers without warning, and a requirement you can only fix by cutting a release is a requirement that will be wrong.

## `report.ok`, not `failed == 0`

```python
if report.ok: ...
```

`ok` requires that at least one capture happened *and* nothing went wrong. A run that captured nothing has an empty problem list, so `failed == 0` is true for it — an empty green is the most misleading result a tool can produce.

## Browser

You need a Chromium. Point `PROOFSHEET_CHROME` at one, or pass `browser=`. [Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/) builds work well and are pinnable.

```python
proofsheet.find_browser()   # raises with instructions if none found
```

## Scope, honestly

**In scope:** anything that renders in a browser engine — web apps, PWAs, and **Capacitor / web-view apps, where the web view genuinely is the app**.

**Out of scope:** true native Swift or Kotlin apps. Those need a simulator, and no amount of Chromium gets you there. Better you know that now than after installing.

## Also available

- Rust: `cargo install proofsheet` (CLI) / `proofsheet-core` (library)
- Node: `npm i @interchained/proofsheet`

Same core, same behaviour, one release.

## Contact

- Web — [interchained.org](https://interchained.org)
- Source — [github.com/interchained/proofsheet](https://github.com/interchained/proofsheet)
- Issues — [github.com/interchained/proofsheet/issues](https://github.com/interchained/proofsheet/issues)
- Email — [dev@interchained.org](mailto:dev@interchained.org)

## License

[BUSL-1.1](https://github.com/interchained/proofsheet/blob/main/LICENSE), converting to **MIT** on 2030-08-22.

Property made in part by **Interchained LLC Labs**.

© 2026 Interchained LLC
