//! Device presets.
//!
//! # Why output pixels come first
//!
//! Apple and Google state their requirements in **output pixels** — "1320 x
//! 2868", "1024 x 500". A browser, however, is driven in **CSS pixels** plus a
//! device pixel ratio. Storing the CSS size and multiplying is the obvious
//! design and it is the wrong one: it lets a preset exist that cannot produce
//! a required size, and the failure only shows up as a rejected upload.
//!
//! So a [`Device`] stores the required output size and *derives* the viewport
//! as `output / scale`. A preset whose output does not divide evenly by its
//! scale is rejected at parse time, which makes "the size we emit is a size
//! the store accepts" a structural property rather than arithmetic somebody
//! has to get right by hand.
//!
//! # Why this is data
//!
//! Stores change these numbers without warning. The table lives in
//! `presets/devices.json` so it can be corrected without cutting a release.
//! The compiled-in copy is only a fallback.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Which store a preset targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Store {
    Apple,
    Play,
    #[default]
    Web,
}

/// How strongly the store asks for this size.
///
/// Free text from the documentation is deliberately narrowed to an enum: an
/// unknown value fails the parse rather than being silently treated as
/// optional, because quietly downgrading a required asset is the expensive
/// failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requirement {
    /// Must be supplied.
    Required,
    /// Required only in a stated situation (e.g. 6.5" when 6.9" is absent).
    Conditional,
    /// Explicitly recommended by the store, not mandatory.
    Recommended,
    /// Accepted, not asked for.
    Optional,
    RequiredIpad,
    RequiredMac,
    RequiredTv,
    RequiredVision,
    RequiredWatch,
}

impl Requirement {
    /// Whether omitting this asset can block or degrade a submission.
    pub fn is_mandatory(self) -> bool {
        !matches!(self, Requirement::Optional | Requirement::Recommended)
    }

    /// Stable snake_case name, matching the JSON representation.
    ///
    /// Written out rather than derived from `Debug`, because lowercasing
    /// `RequiredIpad` silently yields `requiredipad`.
    pub fn as_str(self) -> &'static str {
        match self {
            Requirement::Required => "required",
            Requirement::Conditional => "conditional",
            Requirement::Recommended => "recommended",
            Requirement::Optional => "optional",
            Requirement::RequiredIpad => "required_ipad",
            Requirement::RequiredMac => "required_mac",
            Requirement::RequiredTv => "required_tv",
            Requirement::RequiredVision => "required_vision",
            Requirement::RequiredWatch => "required_watch",
        }
    }
}

impl std::fmt::Display for Requirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One capture target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    /// Stable identifier, used in filenames and receipts.
    pub id: String,
    /// Human label for reports.
    pub label: String,
    /// Required output width in real pixels.
    pub output_width: u32,
    /// Required output height in real pixels.
    pub output_height: u32,
    /// Device pixel ratio the page renders at.
    pub scale: u32,
    /// Emulate a mobile viewport.
    #[serde(default)]
    pub mobile: bool,
    #[serde(default)]
    pub store: Store,
    pub requirement: Requirement,
    /// True only when the numbers were read from official documentation.
    #[serde(default)]
    pub verified: bool,
    /// The documentation URL the numbers came from.
    #[serde(default)]
    pub source: String,
}

impl Device {
    /// The CSS-pixel viewport to drive the browser with.
    ///
    /// Exact by construction: [`parse_presets`] rejects any preset where this
    /// division would not be exact.
    pub fn viewport(&self) -> (u32, u32) {
        (
            self.output_width / self.scale,
            self.output_height / self.scale,
        )
    }

    /// The pixel dimensions the captured image must have.
    pub fn output_size(&self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }

    fn validate(&self) -> Result<()> {
        if self.scale == 0 {
            return Err(Error::Shape(format!("device {}: scale is zero", self.id)));
        }
        if self.output_width == 0 || self.output_height == 0 {
            return Err(Error::Shape(format!(
                "device {}: zero output dimension",
                self.id
            )));
        }
        if self.output_width % self.scale != 0 || self.output_height % self.scale != 0 {
            return Err(Error::Shape(format!(
                "device {}: output {}x{} does not divide by scale {}, so no \
                 integer viewport can produce it",
                self.id, self.output_width, self.output_height, self.scale
            )));
        }
        Ok(())
    }
}

/// The on-disk preset file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetFile {
    pub version: u32,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub verified_on: String,
    pub devices: Vec<Device>,
}

/// Parse and validate a preset file.
pub fn parse_presets(json: &str) -> Result<Vec<Device>> {
    let f: PresetFile = serde_json::from_str(json)?;
    if f.version != 1 {
        return Err(Error::Shape(format!(
            "preset schema version {} is not supported by this build",
            f.version
        )));
    }
    for d in &f.devices {
        d.validate()?;
    }
    let mut ids: Vec<&str> = f.devices.iter().map(|d| d.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    if ids.len() != before {
        return Err(Error::Shape("duplicate device id in preset file".into()));
    }
    Ok(f.devices)
}

/// The compiled-in fallback table.
pub fn builtin() -> Vec<Device> {
    parse_presets(include_str!("../../../presets/devices.json"))
        .expect("built-in presets must parse; enforced by tests")
}

/// Look a device up by id.
pub fn by_id(devices: &[Device], id: &str) -> Option<Device> {
    devices.iter().find(|d| d.id == id).cloned()
}

/// Every device targeting a given store.
pub fn for_store(devices: &[Device], store: Store) -> Vec<Device> {
    devices
        .iter()
        .filter(|d| d.store == store)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_presets_parse() {
        assert!(!builtin().is_empty());
    }

    /// The core invariant. If this fails, some preset cannot produce the
    /// pixel count the store demands, and the harness would emit a
    /// silently-wrong asset.
    #[test]
    fn every_builtin_output_divides_evenly_by_scale() {
        for d in builtin() {
            assert_eq!(
                d.output_width % d.scale,
                0,
                "{}: width {} not divisible by {}",
                d.id,
                d.output_width,
                d.scale
            );
            assert_eq!(
                d.output_height % d.scale,
                0,
                "{}: height {} not divisible by {}",
                d.id,
                d.output_height,
                d.scale
            );
            let (vw, vh) = d.viewport();
            assert_eq!((vw * d.scale, vh * d.scale), d.output_size());
        }
    }

    /// Google Play rejects screenshots outside these bounds. Encoding the
    /// rule as a test means a future edit to the JSON cannot quietly
    /// introduce an unusable Play preset.
    #[test]
    fn play_screenshots_respect_documented_bounds() {
        for d in builtin() {
            if d.store != Store::Play || d.id == "play-feature-graphic" {
                continue;
            }
            let lo = d.output_width.min(d.output_height);
            let hi = d.output_width.max(d.output_height);
            assert!(lo >= 320, "{}: min dimension {lo} below Play's 320", d.id);
            assert!(hi <= 3840, "{}: max dimension {hi} above Play's 3840", d.id);
            assert!(
                hi <= 2 * lo,
                "{}: {hi} exceeds twice {lo}; Play forbids this ratio",
                d.id
            );
        }
    }

    /// Anything claiming a store requirement must cite where the number came
    /// from. This is the guard against inventing authority.
    #[test]
    fn store_presets_are_verified_and_sourced() {
        for d in builtin() {
            if d.store == Store::Web {
                continue;
            }
            assert!(d.verified, "{} targets a store but is not verified", d.id);
            assert!(
                d.source.starts_with("https://"),
                "{} has no source URL",
                d.id
            );
        }
    }

    #[test]
    fn known_apple_sizes_are_present() {
        let b = builtin();
        for id in [
            "apple-iphone-6-9-1320",
            "apple-iphone-6-9-1290",
            "apple-ipad-13-2064",
            "apple-ipad-13-2048",
        ] {
            assert!(by_id(&b, id).is_some(), "missing required preset {id}");
        }
        // 6.9" at 1320x2868 is the current-generation flagship size.
        let d = by_id(&b, "apple-iphone-6-9-1320").unwrap();
        assert_eq!(d.output_size(), (1320, 2868));
        assert_eq!(d.viewport(), (440, 956));
    }

    #[test]
    fn play_feature_graphic_is_exact() {
        let d = by_id(&builtin(), "play-feature-graphic").unwrap();
        assert_eq!(d.output_size(), (1024, 500));
        assert_eq!(d.scale, 1);
    }

    #[test]
    fn indivisible_preset_is_rejected_with_a_useful_message() {
        let js = r#"{"version":1,"devices":[{"id":"bad","label":"bad",
            "output_width":1001,"output_height":2000,"scale":3,
            "requirement":"optional"}]}"#;
        let e = parse_presets(js).unwrap_err().to_string();
        assert!(e.contains("does not divide"), "unhelpful message: {e}");
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let js = r#"{"version":1,"devices":[
            {"id":"a","label":"a","output_width":2,"output_height":2,"scale":1,
             "requirement":"optional"},
            {"id":"a","label":"b","output_width":4,"output_height":4,"scale":1,
             "requirement":"optional"}]}"#;
        assert!(parse_presets(js).is_err());
    }

    #[test]
    fn unknown_requirement_fails_rather_than_defaulting() {
        let js = r#"{"version":1,"devices":[{"id":"a","label":"a",
            "output_width":2,"output_height":2,"scale":1,
            "requirement":"whenever_you_feel_like_it"}]}"#;
        assert!(parse_presets(js).is_err());
    }

    #[test]
    fn future_schema_version_is_rejected_clearly() {
        let e = parse_presets(r#"{"version":99,"devices":[]}"#)
            .unwrap_err()
            .to_string();
        assert!(e.contains("99"));
    }

    /// `as_str` and the serde name must agree, or the JSON a user edits will
    /// not match the label the CLI prints back at them.
    #[test]
    fn requirement_labels_match_their_serde_names() {
        for r in [
            Requirement::Required,
            Requirement::Conditional,
            Requirement::Recommended,
            Requirement::Optional,
            Requirement::RequiredIpad,
            Requirement::RequiredMac,
            Requirement::RequiredTv,
            Requirement::RequiredVision,
            Requirement::RequiredWatch,
        ] {
            let serde_name = serde_json::to_string(&r).unwrap();
            assert_eq!(serde_name, format!("\"{}\"", r.as_str()));
        }
    }

    #[test]
    fn store_filter_partitions_the_table() {
        let b = builtin();
        let n = for_store(&b, Store::Apple).len()
            + for_store(&b, Store::Play).len()
            + for_store(&b, Store::Web).len();
        assert_eq!(n, b.len());
    }
}
