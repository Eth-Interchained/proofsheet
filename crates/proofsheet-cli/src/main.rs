//! `proofsheet` — the command line front end.
//!
//! Argument parsing is hand-rolled. A CLI that exists to be installed
//! everywhere should not pull a parser and its dependency tree along for
//! three subcommands.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use proofsheet_core::{
    capture, cdp, device, Browser, CaptureRequest, Determinism, Device, LaunchOptions, Stability,
    Store, VERSION,
};

const USAGE: &str = "\
proofsheet — exact-pixel store screenshots and deterministic browser runs

USAGE:
    proofsheet <COMMAND> [OPTIONS]

COMMANDS:
    devices                 List device presets
    capture                 Capture screenshots at exact store dimensions
    version                 Print version

DEVICES OPTIONS:
    --store <apple|play|web>    Only show presets for one store
    --mandatory                 Only show presets the store actually requires
    --json                      Emit JSON

CAPTURE OPTIONS:
    --url <URL>                 Page to capture (required)
    --out <DIR>                 Output directory (default: ./proofsheet-out)
    --device <ID>               Preset id; repeatable. Default: all --store
    --store <apple|play|web>    Capture every preset for this store
    --seed <N>                  Determinism seed (default: 42)
    --locale <TAG>              Locale override (default: en-US)
    --presets <FILE>            Load device table from FILE instead of built-in
    --json                      Emit a JSON manifest to stdout

ENVIRONMENT:
    PROOFSHEET_CHROME           Path to a Chromium/Chrome binary
";

struct Args {
    command: String,
    flags: BTreeMap<String, Vec<String>>,
}

impl Args {
    fn parse(argv: &[String]) -> Args {
        let mut flags: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let command = argv.first().cloned().unwrap_or_default();
        let mut i = 1;
        while i < argv.len() {
            let a = &argv[i];
            if let Some(name) = a.strip_prefix("--") {
                let (name, inline) = match name.split_once('=') {
                    Some((n, v)) => (n, Some(v.to_string())),
                    None => (name, None),
                };
                let value = match inline {
                    Some(v) => v,
                    None => {
                        // A flag followed by another flag (or nothing) is a
                        // boolean; treat its presence as "true".
                        if i + 1 < argv.len() && !argv[i + 1].starts_with("--") {
                            i += 1;
                            argv[i].clone()
                        } else {
                            "true".to_string()
                        }
                    }
                };
                flags.entry(name.to_string()).or_default().push(value);
            }
            i += 1;
        }
        Args { command, flags }
    }

    fn one(&self, k: &str) -> Option<&str> {
        self.flags.get(k).and_then(|v| v.last()).map(String::as_str)
    }

    fn many(&self, k: &str) -> Vec<String> {
        self.flags.get(k).cloned().unwrap_or_default()
    }

    fn has(&self, k: &str) -> bool {
        self.flags.contains_key(k)
    }
}

fn parse_store(s: &str) -> Option<Store> {
    match s.to_ascii_lowercase().as_str() {
        "apple" | "ios" | "appstore" => Some(Store::Apple),
        "play" | "android" | "google" => Some(Store::Play),
        "web" => Some(Store::Web),
        _ => None,
    }
}

fn load_devices(args: &Args) -> Result<Vec<Device>, String> {
    match args.one("presets") {
        Some(path) => {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            device::parse_presets(&text).map_err(|e| e.to_string())
        }
        None => Ok(device::builtin()),
    }
}

/// Resolve which devices a capture run should target.
///
/// An unknown `--device` id is an error, never a skip: silently capturing a
/// smaller set than the user asked for produces an incomplete upload that
/// looks successful.
fn select_devices(args: &Args, all: &[Device]) -> Result<Vec<Device>, String> {
    let ids = args.many("device");
    if !ids.is_empty() {
        let mut out = Vec::new();
        for id in ids {
            match device::by_id(all, &id) {
                Some(d) => out.push(d),
                None => return Err(format!("unknown device id: {id}")),
            }
        }
        return Ok(out);
    }
    if let Some(s) = args.one("store") {
        let store = parse_store(s).ok_or_else(|| format!("unknown store: {s}"))?;
        let out = device::for_store(all, store);
        if out.is_empty() {
            return Err(format!("no presets for store {s}"));
        }
        return Ok(out);
    }
    Err("specify --device <id> (repeatable) or --store <apple|play|web>".into())
}

fn cmd_devices(args: &Args) -> Result<(), String> {
    let all = load_devices(args)?;
    let filtered: Vec<&Device> = all
        .iter()
        .filter(|d| match args.one("store") {
            Some(s) => parse_store(s) == Some(d.store),
            None => true,
        })
        .filter(|d| !args.has("mandatory") || d.requirement.is_mandatory())
        .collect();

    if args.has("json") {
        let json =
            serde_json::to_string_pretty(&filtered).map_err(|e| format!("serialize: {e}"))?;
        println!("{json}");
        return Ok(());
    }

    println!(
        "{:<28}{:>12}{:>12}{:>4}  {:<16}VERIFIED",
        "ID", "OUTPUT", "VIEWPORT", "DPR", "REQUIREMENT"
    );
    for d in &filtered {
        let (ow, oh) = d.output_size();
        let (vw, vh) = d.viewport();
        println!(
            "{:<28}{:>12}{:>12}{:>4}  {:<16}{}",
            d.id,
            format!("{ow}x{oh}"),
            format!("{vw}x{vh}"),
            d.scale,
            d.requirement,
            if d.verified { "yes" } else { "no" }
        );
    }
    println!("\n{} presets", filtered.len());
    Ok(())
}

fn cmd_capture(args: &Args) -> Result<bool, String> {
    let url = args
        .one("url")
        .ok_or_else(|| "--url is required".to_string())?;
    let out_dir = PathBuf::from(args.one("out").unwrap_or("./proofsheet-out"));
    let all = load_devices(args)?;
    let targets = select_devices(args, &all)?;

    let seed: u64 = match args.one("seed") {
        Some(s) => s.parse().map_err(|_| format!("bad --seed: {s}"))?,
        None => 42,
    };
    let mut det = Determinism::default().with_seed(seed);
    if let Some(l) = args.one("locale") {
        det = det.with_locale(l);
    }
    let stability = Stability::default();

    let binary = cdp::find_browser(None).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;

    let mut results = Vec::new();
    let mut all_exact = true;

    for d in &targets {
        // A fresh browser per device: emulation overrides accumulate on a
        // session, and a leaked override from a previous device is exactly
        // the kind of bug that produces a plausible, wrong image.
        let opts = LaunchOptions::new(&binary);
        let mut browser = Browser::launch(&opts).map_err(|e| e.to_string())?;
        let req = CaptureRequest {
            url,
            device: d,
            determinism: &det,
            stability: &stability,
        };
        let (cap, bytes) = capture(&mut browser, &req).map_err(|e| e.to_string())?;
        let path = out_dir.join(format!("{}.png", d.id));
        std::fs::write(&path, &bytes)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;

        if !cap.exact {
            all_exact = false;
        }
        if !args.has("json") {
            println!(
                "{:<28} {:>11}  {}  {}",
                cap.device_id,
                format!("{}x{}", cap.actual.0, cap.actual.1),
                if cap.exact { "exact " } else { "WRONG " },
                &cap.sha256[..16]
            );
        }
        results.push(cap);
    }

    if args.has("json") {
        let manifest = serde_json::json!({
            "proofsheet": VERSION,
            "url": url,
            "seed": seed,
            "locale": det.locale,
            "captures": results,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?
        );
    } else {
        println!("\n{} captures -> {}", results.len(), out_dir.display());
        if !all_exact {
            println!("SOME CAPTURES ARE THE WRONG SIZE — do not upload these");
        }
    }
    Ok(all_exact)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        print!("{USAGE}");
        return ExitCode::from(2);
    }
    let args = Args::parse(&argv);
    let outcome = match args.command.as_str() {
        "devices" => cmd_devices(&args).map(|_| true),
        "capture" => cmd_capture(&args),
        "version" => {
            println!("proofsheet {VERSION}");
            Ok(true)
        }
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(true)
        }
        other => Err(format!("unknown command: {other}\n\n{USAGE}")),
    };
    match outcome {
        Ok(true) => ExitCode::SUCCESS,
        // A run that completed but produced an off-size asset must not exit 0;
        // CI has to be able to catch it.
        Ok(false) => ExitCode::from(1),
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Args {
        Args::parse(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_repeated_flags() {
        let a = args(&["capture", "--device", "x", "--device", "y"]);
        assert_eq!(a.command, "capture");
        assert_eq!(a.many("device"), vec!["x", "y"]);
    }

    #[test]
    fn parses_equals_form() {
        let a = args(&["capture", "--url=https://example.com"]);
        assert_eq!(a.one("url"), Some("https://example.com"));
    }

    #[test]
    fn bare_flag_is_boolean() {
        let a = args(&["devices", "--json", "--store", "apple"]);
        assert!(a.has("json"));
        assert_eq!(a.one("store"), Some("apple"));
    }

    #[test]
    fn store_aliases_resolve() {
        assert_eq!(parse_store("ios"), Some(Store::Apple));
        assert_eq!(parse_store("android"), Some(Store::Play));
        assert_eq!(parse_store("nonsense"), None);
    }

    #[test]
    fn unknown_device_id_is_an_error_not_a_silent_skip() {
        let all = device::builtin();
        let a = args(&["capture", "--device", "no-such-device"]);
        let e = select_devices(&a, &all).unwrap_err();
        assert!(e.contains("unknown device id"), "{e}");
    }

    #[test]
    fn store_selection_returns_a_nonempty_set() {
        let all = device::builtin();
        let a = args(&["capture", "--store", "apple"]);
        assert!(!select_devices(&a, &all).unwrap().is_empty());
    }

    #[test]
    fn selection_requires_an_explicit_choice() {
        let all = device::builtin();
        assert!(select_devices(&args(&["capture"]), &all).is_err());
    }
}
