//! Capturing one image, at exactly the size a store demands.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::cdp::Browser;
use crate::determinism::Determinism;
use crate::device::Device;
use crate::error::{Error, Result};
use crate::png;

/// How long to let the page settle before capturing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stability {
    /// Wait for `document.fonts.ready`. A capture taken mid font-swap shows
    /// fallback metrics and is the single most common source of screenshot
    /// churn.
    pub fonts: bool,
    /// Wait for every `<img>` to finish decoding, not merely to load.
    pub images: bool,
    /// Require this many consecutive animation frames with no DOM mutation.
    pub quiet_frames: u32,
    /// Give up waiting after this many milliseconds and capture anyway.
    pub timeout_ms: u32,
}

impl Default for Stability {
    fn default() -> Self {
        Stability {
            fonts: true,
            images: true,
            quiet_frames: 2,
            timeout_ms: 10_000,
        }
    }
}

/// What the page actually saw, read back from the page itself.
///
/// # Why this is recorded
///
/// Dimensions alone cannot tell you whether a screenshot is right. A capture
/// can be exactly 1320x2868 and still show a desktop layout scaled into a
/// phone frame, because a page served to a desktop User-Agent may declare
/// `<meta name="viewport" content="width=1120">` and Chrome honours it.
/// Every size assertion passes; the image is useless.
///
/// So the run records the viewport and touch points the page reported, and
/// callers can assert on the environment rather than only on the output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    /// `window.innerWidth` as the page saw it.
    pub inner_width: u32,
    /// `window.innerHeight` as the page saw it.
    pub inner_height: u32,
    /// `devicePixelRatio` as the page saw it.
    pub device_pixel_ratio: u32,
    /// `navigator.maxTouchPoints`.
    pub touch_points: u32,
    /// Whether the layout viewport matched what was requested. False means
    /// the page overrode it, which usually means content negotiation served
    /// the wrong layout.
    pub viewport_honoured: bool,
}

/// The result of one capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capture {
    pub device_id: String,
    /// What the store requires.
    pub expected: (u32, u32),
    /// What the browser actually produced.
    pub actual: (u32, u32),
    /// Content address of the image bytes.
    pub sha256: String,
    pub bytes: usize,
    /// Whether `actual == expected`. A false here is a hard failure, not a
    /// warning: an off-size asset is rejected at upload.
    pub exact: bool,
    /// What the page reported about itself. `None` if the probe failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
}

/// Everything needed to drive one capture.
#[derive(Debug, Clone)]
pub struct CaptureRequest<'a> {
    pub url: &'a str,
    pub device: &'a Device,
    pub determinism: &'a Determinism,
    pub stability: &'a Stability,
}

/// Build the JS that resolves once the page has stopped moving.
///
/// Note the deliberate use of real timers here rather than the virtualised
/// clock: this runs in the harness's frame of reference, not the page's.
fn stability_script(s: &Stability) -> String {
    format!(
        r#"new Promise((resolve) => {{
  const deadline = Date.now() + {timeout};
  const done = (why) => resolve(why);
  const waits = [];
  if ({fonts} && document.fonts && document.fonts.ready) {{
    waits.push(document.fonts.ready.catch(() => {{}}));
  }}
  if ({images}) {{
    const imgs = Array.from(document.images || []);
    waits.push(Promise.all(imgs.map((i) =>
      (i.decode ? i.decode().catch(() => {{}}) : Promise.resolve())
    )));
  }}
  Promise.all(waits).then(() => {{
    let quiet = 0;
    let dirty = false;
    const obs = new MutationObserver(() => {{ dirty = true; }});
    obs.observe(document.documentElement, {{
      subtree: true, childList: true, attributes: true, characterData: true
    }});
    const tick = () => {{
      if (Date.now() > deadline) {{ obs.disconnect(); return done('timeout'); }}
      if (dirty) {{ dirty = false; quiet = 0; }} else {{ quiet++; }}
      if (quiet >= {frames}) {{ obs.disconnect(); return done('quiet'); }}
      setTimeout(tick, 16);
    }};
    tick();
  }});
}})"#,
        timeout = s.timeout_ms,
        fonts = s.fonts,
        images = s.images,
        frames = s.quiet_frames.max(1),
    )
}

/// Capture a single image.
///
/// Order matters and is the whole trick: metrics are applied **before**
/// navigation so the first layout already happens at the target size. Setting
/// them afterwards produces a reflow, and anything that measured the viewport
/// during initial layout is now wrong.
pub fn capture(browser: &mut Browser, req: &CaptureRequest<'_>) -> Result<(Capture, Vec<u8>)> {
    let (vw, vh) = req.device.viewport();

    browser.call("Page.enable", json!({}))?;
    browser.call("Runtime.enable", json!({}))?;

    browser.call(
        "Emulation.setDeviceMetricsOverride",
        json!({
            "width": vw,
            "height": vh,
            "deviceScaleFactor": req.device.scale,
            "mobile": req.device.mobile,
        }),
    )?;

    // Metrics alone are NOT device emulation.
    //
    // A page served to a desktop User-Agent may declare
    // `<meta name="viewport" content="width=1120">`, and Chrome honours that
    // whenever `mobile` is set -- so the layout viewport becomes 1120 CSS px
    // and the desktop layout is merely scaled into a phone-sized frame. The
    // pixel count is right and the content is wrong, which is the single most
    // dangerous failure this tool can have: it looks like a success.
    //
    // Measured on a real site, identical metrics, changing only these two
    // calls: innerWidth 1120 -> 440, maxTouchPoints 0 -> 5, and the server
    // returned different HTML.
    let platform = req.device.platform;
    if let Some(ua) = platform.user_agent() {
        browser
            .call(
                "Emulation.setUserAgentOverride",
                json!({
                    "userAgent": ua,
                    "platform": platform.ch_platform(),
                    // Client Hints as well as the UA string: modern sites
                    // branch on Sec-CH-UA-Mobile, and overriding only one
                    // leaves the page half-convinced it is on a phone.
                    "userAgentMetadata": {
                        "platform": platform.ch_platform(),
                        "platformVersion": "",
                        "architecture": "",
                        "model": "",
                        "mobile": platform.ch_mobile(),
                        "brands": [],
                    },
                }),
            )
            .or_else(swallow_unsupported)?;
    }
    let touch = platform.touch_points();
    browser
        .call(
            "Emulation.setTouchEmulationEnabled",
            json!({ "enabled": touch > 0, "maxTouchPoints": touch.max(1) }),
        )
        .or_else(swallow_unsupported)?;

    // Locale and timezone are set through CDP rather than script because the
    // script-level overrides are trivially detectable and do not affect
    // Intl's internal data.
    browser
        .call(
            "Emulation.setTimezoneOverride",
            json!({ "timezoneId": req.determinism.timezone }),
        )
        .or_else(swallow_unsupported)?;
    browser
        .call(
            "Emulation.setLocaleOverride",
            json!({ "locale": req.determinism.locale }),
        )
        .or_else(swallow_unsupported)?;

    browser.call(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": req.determinism.preamble() }),
    )?;

    browser.call("Page.navigate", json!({ "url": req.url }))?;

    browser.call(
        "Runtime.evaluate",
        json!({
            "expression": stability_script(req.stability),
            "awaitPromise": true,
            "returnByValue": true,
        }),
    )?;

    // Read back what the page actually saw, before capturing. This is the
    // check that catches "right pixels, wrong layout".
    let environment = probe_environment(browser, vw, vh).ok();

    let shot = browser.call(
        "Page.captureScreenshot",
        json!({ "format": "png", "captureBeyondViewport": false }),
    )?;
    let b64 = shot
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Shape("captureScreenshot returned no data".into()))?;
    let bytes = B64
        .decode(b64)
        .map_err(|e| Error::Shape(format!("screenshot was not valid base64: {e}")))?;

    let actual = png::dimensions(&bytes)?;
    let expected = req.device.output_size();

    let mut h = Sha256::new();
    h.update(&bytes);
    let sha256 = format!("{:x}", h.finalize());

    Ok((
        Capture {
            device_id: req.device.id.clone(),
            expected,
            actual,
            sha256,
            bytes: bytes.len(),
            exact: actual == expected,
            environment,
        },
        bytes,
    ))
}

/// Ask the page what viewport and input capabilities it believes it has.
fn probe_environment(browser: &mut Browser, want_w: u32, want_h: u32) -> Result<Environment> {
    let v = browser.call(
        "Runtime.evaluate",
        json!({
            "expression": "({w:innerWidth,h:innerHeight,d:devicePixelRatio,\
                            t:(navigator.maxTouchPoints||0)})",
            "returnByValue": true,
        }),
    )?;
    let obj = v
        .get("result")
        .and_then(|r| r.get("value"))
        .ok_or_else(|| Error::Shape("environment probe returned nothing".into()))?;
    let num = |k: &str| obj.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0) as u32;
    let inner_width = num("w");
    let inner_height = num("h");
    Ok(Environment {
        inner_width,
        inner_height,
        device_pixel_ratio: num("d"),
        touch_points: num("t"),
        // A page that overrides the layout viewport via meta-viewport is
        // telling us it did not accept the device we claimed to be.
        viewport_honoured: inner_width == want_w && inner_height == want_h,
    })
}

/// Some `Emulation.*` overrides are unavailable in older or reduced builds
/// (`chrome-headless-shell` in particular). A missing locale override should
/// degrade the run, not end it — but anything else must still propagate.
fn swallow_unsupported(e: Error) -> Result<serde_json::Value> {
    match &e {
        Error::Cdp { message, .. }
            if message.contains("wasn't found") || message.contains("not supported") =>
        {
            Ok(serde_json::Value::Null)
        }
        _ => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stability_script_honours_its_flags() {
        let s = Stability {
            fonts: false,
            images: true,
            quiet_frames: 3,
            timeout_ms: 500,
        };
        let js = stability_script(&s);
        assert!(js.contains("if (false && document.fonts"));
        assert!(js.contains("if (true)"));
        assert!(js.contains("quiet >= 3"));
        assert!(js.contains("Date.now() + 500"));
    }

    #[test]
    fn quiet_frames_never_degenerates_to_zero() {
        // Zero would make the loop resolve before observing anything.
        let s = Stability {
            quiet_frames: 0,
            ..Default::default()
        };
        assert!(stability_script(&s).contains("quiet >= 1"));
    }

    #[test]
    fn unsupported_domain_errors_are_swallowed_but_others_are_not() {
        let ok = swallow_unsupported(Error::Cdp {
            method: "Emulation.setLocaleOverride".into(),
            message: "'Emulation.setLocaleOverride' wasn't found".into(),
        });
        assert!(ok.is_ok());

        let bad = swallow_unsupported(Error::Cdp {
            method: "Page.navigate".into(),
            message: "Cannot navigate to invalid URL".into(),
        });
        assert!(bad.is_err(), "real errors must not be swallowed");
    }
}
