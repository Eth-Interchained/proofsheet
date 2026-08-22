# proofsheet

**Exact-pixel store screenshots and deterministic browser runs. Local-first, model-agnostic, no cloud.**

A *proof sheet* is the contact sheet a photographer reviews before choosing which frames to print. This does both halves of that: it **proves** (deterministic runs, verifiable receipts) and it produces the **sheet** (every screenshot your app store asks for, at exactly the size it asks for).

```
proofsheet capture --url https://your.app --store apple --out ./shots
```

```
apple-iphone-6-9-1320          1320x2868  exact   1487bb3d1bc8ece2
apple-iphone-6-9-1290          1290x2796  exact   8d4ed8b534ba232e
apple-ipad-13-2064             2064x2752  exact   137db41f829b6f90
```

---

## Why

Store screenshot sets rot. They get taken by hand, at different moments, on different builds — one shot says 3:47 and the next says 9:12, a list reshuffles between frames, "2 hours ago" quietly becomes "3 days ago". Then a redesign lands and somebody spends two days redoing all of them.

proofsheet makes the whole set a build artifact. Same seed, same output, byte for byte. Rerun after a redesign and the only things that changed are the things you changed.

## What makes the pixels exact

Two decisions, both structural rather than careful:

**Output pixels are the source of truth.** Apple and Google publish requirements in output pixels — `1320 x 2868`, `1024 x 500`. Browsers are driven in CSS pixels plus a device pixel ratio. Storing the CSS size and multiplying is the obvious design and it is the wrong one, because it permits a preset that cannot produce a required size — and you find out at upload. So a preset stores the **required output size** and derives the viewport as `output / scale`. Any preset where that division isn't exact is rejected at parse time.

**Metrics are applied before layout.** `Emulation.setDeviceMetricsOverride` runs before navigation, so first layout already happens at the target size, and `Page.captureScreenshot` emits exactly those pixels. Nothing is resized, cropped, or padded after the fact. The image is born the right size.

## What makes runs deterministic

A preamble is injected via `Page.addScriptToEvaluateOnNewDocument`, so it runs before any page script on every document. It replaces `Math.random` with a seeded PRNG, freezes `Date.now` and `performance.now`, drives `requestAnimationFrame` on a fixed virtual step, and routes `crypto.getRandomValues` through the seeded stream. Locale and timezone are pinned through CDP rather than script, because the script-level overrides don't reach Intl's internal data.

Verified in both directions: same seed produces byte-identical PNGs across independent browser launches, and a different seed produces different bytes. The second half is the part that makes the first half evidence instead of a green check.

## Device presets

46 presets, every store size read from official documentation on 2026-08-22:

- **Apple** — iPhone 6.9″/6.5″/6.3″/6.1″/5.5″/4.7″, iPad 13″/11″/10.5″/9.7″, Mac, Apple TV, Vision Pro, Apple Watch
  <br><sub>[App Store Connect Help → Screenshot specifications](https://developer.apple.com/help/app-store-connect/reference/screenshot-specifications/)</sub>
- **Google Play** — feature graphic, phone, 7″/10″ tablet, Wear OS, Automotive, TV banner and screenshot
  <br><sub>[Play Console Help → Add preview assets](https://support.google.com/googleplay/android-developer/answer/9866151)</sub>

Presets are **data**, in `presets/devices.json`. Stores change these numbers without warning, and a requirement that can only be corrected by cutting a release is a requirement that will be wrong. Point `--presets` at your own file to override. Every entry carries the URL it came from, and the test suite refuses to let an entry claim `verified` without one.

```
proofsheet devices --store apple --mandatory
```

## Install

```
cargo install proofsheet          # crates.io
npm  i -g @interchained/proofsheet # npm      (planned)
pip  install proofsheet            # PyPI     (planned)
```

You also need a Chromium. Point `PROOFSHEET_CHROME` at one, or let proofsheet fetch a pinned [Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/) build.

## Scope, honestly

**In scope:** anything that renders in a browser engine — web apps, PWAs, and **Capacitor / web-view apps, where the web view genuinely is the app**.

**Out of scope:** true native Swift or Kotlin apps. Those need a simulator, and no amount of Chromium gets you there. If that's what you have, this is the wrong tool and you should know that before you install it rather than after.

## Status

v0.1 is the capture path: exact-pixel captures, determinism, device presets, CLI.

Next, in order: hash-chained receipts in [NEDB](https://github.com/Eth-Interchained/nedb) so every image is content-addressed against the state and commit that produced it; the locale × theme matrix; a scoped compositor for device frames and caption bands; then the agent-driven test path — Explorer, Oracle, and a Reducer that shrinks a failing run to its minimal reproducing sequence.

Dependencies are deliberately few: no async runtime, no browser automation framework. The WebSocket and CDP client are about 500 lines of `std`.

## License

[BUSL-1.1](LICENSE), converting to **MIT** on 2030-08-22. Production use is granted; offering proofsheet itself as a hosted service is not.

© 2026 Interchained LLC
