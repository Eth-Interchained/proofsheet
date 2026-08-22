//! The determinism preamble.
//!
//! Injected via `Page.addScriptToEvaluateOnNewDocument`, so it runs before any
//! page script on every document — including ones created by navigation.
//!
//! Why this exists: store screenshot sets rot because they are taken by hand
//! at different moments. One shot says 3:47, the next says 9:12; a list
//! reshuffles; "2 hours ago" becomes "3 days ago". Freezing the clock and
//! seeding the PRNG makes a rerun reproduce the entire set byte for byte,
//! so the only diffs are the ones you meant to make.
//!
//! Validated empirically before this code existed: with the preamble, two
//! independent browser launches produced identical PNG hashes across five
//! device sizes; with it disabled, the same page produced different bytes.
//! The negative control is the part that makes that evidence rather than a
//! green check.

/// Knobs for the injected preamble.
#[derive(Debug, Clone, PartialEq)]
pub struct Determinism {
    /// Seed for the replacement PRNG.
    pub seed: u64,
    /// The instant `Date.now()` reports, in milliseconds since the epoch.
    pub epoch_ms: i64,
    /// Fixed timezone, e.g. `UTC`. Applied via CDP, not script.
    pub timezone: String,
    /// Fixed locale, e.g. `en-US`. Applied via CDP, not script.
    pub locale: String,
    /// Virtual milliseconds advanced per animation frame.
    pub frame_ms: f64,
}

impl Default for Determinism {
    fn default() -> Self {
        Determinism {
            seed: 42,
            // A fixed, boring instant. Chosen once and never changed, because
            // changing it would churn every committed screenshot everywhere.
            epoch_ms: 1_750_000_000_000,
            timezone: "UTC".into(),
            locale: "en-US".into(),
            frame_ms: 1000.0 / 60.0,
        }
    }
}

impl Determinism {
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = locale.into();
        self
    }

    /// Render the JavaScript preamble for these settings.
    pub fn preamble(&self) -> String {
        // mulberry32: tiny, fast, and stable across implementations, which
        // matters because the Rust side may need to predict the same stream.
        format!(
            r#"(() => {{
  let s = {seed} >>> 0;
  Math.random = function () {{
    s |= 0; s = (s + 0x6D2B79F5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }};
  const T0 = {epoch};
  const RealDate = Date;
  function FrozenDate(...a) {{
    if (!(this instanceof FrozenDate)) return new RealDate(T0).toString();
    return a.length ? new RealDate(...a) : new RealDate(T0);
  }}
  FrozenDate.prototype = RealDate.prototype;
  FrozenDate.now = () => T0;
  FrozenDate.parse = RealDate.parse;
  FrozenDate.UTC = RealDate.UTC;
  Object.defineProperty(FrozenDate, 'name', {{ value: 'Date' }});
  window.Date = FrozenDate;

  let perf = 0;
  performance.now = () => perf;

  const FRAME = {frame};
  window.requestAnimationFrame = (cb) => {{
    perf += FRAME;
    const t = perf;
    return setTimeout(() => cb(t), 0);
  }};
  window.cancelAnimationFrame = (h) => clearTimeout(h);

  // crypto.getRandomValues is a second entropy source that would otherwise
  // leak nondeterminism into anything generating ids.
  if (window.crypto && crypto.getRandomValues) {{
    crypto.getRandomValues = (arr) => {{
      for (let i = 0; i < arr.length; i++) {{
        arr[i] = Math.floor(Math.random() * 256);
      }}
      return arr;
    }};
  }}
}})();"#,
            seed = self.seed,
            epoch = self.epoch_ms,
            frame = self.frame_ms,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preamble_embeds_its_settings() {
        let d = Determinism::default().with_seed(7);
        let js = d.preamble();
        assert!(js.contains("let s = 7 >>> 0"));
        assert!(js.contains("const T0 = 1750000000000"));
    }

    #[test]
    fn preamble_overrides_every_known_entropy_source() {
        let js = Determinism::default().preamble();
        for sym in [
            "Math.random",
            "Date.now",
            "performance.now",
            "requestAnimationFrame",
            "getRandomValues",
        ] {
            assert!(js.contains(sym), "preamble does not override {sym}");
        }
    }

    #[test]
    fn default_epoch_is_pinned() {
        // If this ever changes, every committed screenshot in every
        // downstream repo churns. Treat a failure here as a deliberate
        // decision, not a test to update.
        assert_eq!(Determinism::default().epoch_ms, 1_750_000_000_000);
    }
}
