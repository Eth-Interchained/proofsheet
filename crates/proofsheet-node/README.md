# @interchained/proofsheet

**Exact-pixel App Store and Google Play screenshots from a real browser. Deterministic, local-first, no cloud.**

**Source:** [github.com/interchained/proofsheet](https://github.com/interchained/proofsheet)

```bash
npm i @interchained/proofsheet
```

```js
import { capture } from '@interchained/proofsheet'

const report = await capture({
  url: 'https://your.app',
  outDir: './shots',
  store: 'apple',
})

console.log(`${report.exact} exact, ${report.offSize} off-size, ${report.failed} failed`)
if (!report.ok) process.exit(1)
```

---

## Point it at anything a browser can open

There is no "local mode" and no "remote mode" — `url` takes a URL. All four of these are ordinary usage:

```js
await capture({ url: 'http://localhost:5173', store: 'apple', outDir: './shots' })      // dev server
await capture({ url: `file://${process.cwd()}/dist/index.html`, store: 'apple' })       // static build
await capture({ url: 'https://pr-482.preview.example.com', store: 'apple' })            // preview deploy
await capture({ url: 'https://your.app', store: 'apple' })                              // production
```

**Prefer localhost.** Not as a fallback — as the default:

- **You capture before you ship.** The set is built from the branch you're about to release, so it can *gate* the release rather than document it afterwards.
- **CI needs no deploy and no public URL.** Start your dev server, capture, upload.
- **It's hermetic.** A live domain drags in CDN state, cookie banners, A/B buckets and analytics — all of which move between runs and destroy byte-identical determinism. Localhost doesn't.
- **It's faster.** Measured on the same page: `366ms` local vs `1.6s` over the network. Across a 44-device matrix that's ~25 seconds versus a couple of minutes.
- **It works on apps that aren't public yet**, or are behind auth.

A complete pre-publish script:

```js
import { spawn } from 'node:child_process'
import { capture } from '@interchained/proofsheet'

const server = spawn('npm', ['run', 'preview'], { stdio: 'inherit' })
try {
  // Wait for the port rather than sleeping a hopeful number of seconds.
  const url = 'http://localhost:4173'
  for (let i = 0; i < 60; i++) {
    try { await fetch(url); break } catch { await new Promise(r => setTimeout(r, 250)) }
  }

  const report = await capture({ url, store: 'apple', outDir: './shots' })
  console.log(`${report.exact} exact, ${report.offSize} off-size, ${report.failed} failed`)
  if (!report.ok) process.exitCode = 1
} finally {
  server.kill()
}
```

Two things worth knowing before they bite you:

**`file://` is not a real origin.** Fastest path, fine for genuinely static pages, but `fetch`, service workers, ES module imports and anything CORS-sensitive behave differently there than in production. If your app does real work, run the dev server and point at `localhost`.

**Inside Docker, `localhost` means the container.** If this runs in a container while your dev server runs on the host, use `host.docker.internal` or `--network host`.

## Why

Store screenshot sets rot. They get taken by hand, at different moments, on different builds — one shot says 3:47 and the next says 9:12, a list reshuffles between frames, "2 hours ago" becomes "3 days ago". Then a redesign lands and somebody spends two days redoing all of them.

proofsheet makes the set a build artifact. Same seed, same bytes.

## Exact pixels, structurally

Apple and Google publish requirements in **output pixels** — `1320 × 2868`, `1024 × 500`. A browser is driven in **CSS pixels** plus a device pixel ratio. Storing the CSS size and multiplying permits a preset that cannot produce a required size, and you find out at upload.

So a preset stores the required output size and *derives* the viewport as `output / scale`. Any preset where that division isn't exact is rejected before a browser starts.

```js
import { devices } from '@interchained/proofsheet'

const [d] = devices('apple')
d.output_width / d.scale   // integer, always
d.source                   // the Apple doc URL this came from
```

## Determinism

A preamble is injected before any page script on every document: seeded PRNG, frozen clock, virtual `requestAnimationFrame`, seeded `crypto.getRandomValues`. Locale and timezone are pinned through CDP rather than script, because the script-level overrides don't reach Intl's internal data.

Verified in both directions — the same seed produces byte-identical PNGs across independent browser launches, and a different seed produces different bytes. The second half is what makes the first half evidence.

```js
const a = await capture({ url, devices: ['apple-iphone-6-9-1320'], seed: 42 })
const b = await capture({ url, devices: ['apple-iphone-6-9-1320'], seed: 42 })
a.results[0].capture.sha256 === b.results[0].capture.sha256  // true
```

## `report.ok`, not `report.failed === 0`

`ok` requires that at least one capture happened *and* nothing went wrong. A run that captured nothing has an empty problem list, so `failed === 0` is true for it — an empty green is the most misleading result a tool can produce.

## Runs off the main thread

`capture()` dispatches to libuv's thread pool and returns a promise. A full store matrix is 44 browser launches; it never blocks your event loop.

## Presets

46 presets, every store size read from official documentation on 2026-08-22, each carrying its source URL.

- **Apple** — iPhone 6.9″/6.5″/6.3″/6.1″/5.5″/4.7″, iPad 13″/11″/10.5″/9.7″, Mac, Apple TV, Vision Pro, Apple Watch
- **Google Play** — feature graphic, phone, 7″/10″ tablet, Wear OS, Automotive, TV

Pass `presets: 'my-devices.json'` to use your own table. Stores change these numbers without warning.

## Browser

You need a Chromium. Set `PROOFSHEET_CHROME`, or pass `browser`. [Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/) builds work well and are pinnable.

```js
import { findBrowser } from '@interchained/proofsheet'
findBrowser()   // throws with instructions if none found
```

## Scope, honestly

**In scope:** anything that renders in a browser engine — web apps, PWAs, and **Capacitor / web-view apps, where the web view genuinely is the app**.

**Out of scope:** true native Swift or Kotlin apps. Those need a simulator. Better you know now than after installing.

## Also available

- Rust: `cargo install proofsheet` (CLI) / `proofsheet-core` (library)
- Python: `pip install proofsheet`

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
