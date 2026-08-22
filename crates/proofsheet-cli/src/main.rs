//! `proofsheet` — the command line front end.
//!
//! Argument parsing is hand-rolled. A CLI that exists to be installed
//! everywhere should not pull a parser and its dependency tree along for
//! three subcommands.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use proofsheet_core::{
    cdp, device, progress, run, Determinism, Device, DeviceEvent, Progress, RunOptions, Stability,
    Store, Summary, VERSION,
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
    --fail-fast                 Stop at the first failure
    --quiet                     Suppress progress output
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

/// Renders run progress to a terminal.
///
/// Writes to stderr so `--json` on stdout stays machine-parseable while the
/// human still sees movement. Redraws in place when attached to a TTY and
/// falls back to plain lines when piped, because a log full of carriage
/// returns is worse than no progress at all.
struct TerminalProgress {
    tty: bool,
    quiet: bool,
    width: usize,
}

impl TerminalProgress {
    fn new(quiet: bool) -> Self {
        TerminalProgress {
            tty: is_tty(),
            quiet,
            width: 24,
        }
    }

    fn clear_line(&self) {
        if self.tty {
            eprint!("\r\x1b[2K");
        }
    }
}

impl Progress for TerminalProgress {
    fn run_started(&mut self, total: usize, url: &str) {
        if self.quiet {
            return;
        }
        eprintln!("capturing {url}");
        eprintln!("{total} device{}", if total == 1 { "" } else { "s" });
    }

    fn device_started(&mut self, index: usize, total: usize, device: &Device) {
        if self.quiet || !self.tty {
            return;
        }
        eprint!(
            "\r\x1b[2K[{}] {}/{}  {}",
            progress::bar(index - 1, total, self.width),
            index,
            total,
            device.id
        );
    }

    fn device_finished(&mut self, e: &DeviceEvent<'_>) {
        if self.quiet {
            return;
        }
        self.clear_line();
        let size = e
            .capture
            .map(|c| format!("{}x{}", c.actual.0, c.actual.1))
            .unwrap_or_else(|| "-".into());
        let digest = e
            .capture
            .map(|c| c.sha256[..16].to_string())
            .unwrap_or_default();
        eprintln!(
            "  {:>3}/{:<3} {:<28} {:>11}  {:<9} {:>7}  {}",
            e.index,
            e.total,
            e.device.id,
            size,
            e.outcome.as_str(),
            progress::human(e.elapsed),
            digest
        );
        if let Some(msg) = e.error {
            eprintln!("        {msg}");
        }
        if let Some(c) = e.capture {
            if !c.exact {
                eprintln!(
                    "        wanted {}x{}, got {}x{}",
                    c.expected.0, c.expected.1, c.actual.0, c.actual.1
                );
            }
            // The image is the right size but the page laid out at a
            // different width, so this is a picture of the wrong layout.
            // Almost always a missing meta viewport, occasionally content
            // negotiation ignoring the emulated device.
            if let Some(env) = &c.environment {
                if !env.viewport_honoured && e.device.mobile {
                    let (vw, vh) = e.device.viewport();
                    eprintln!(
                        "        page laid out at {}x{}, not {}x{} -- add \
                         <meta name=\"viewport\" content=\"width=device-width, \
                         initial-scale=1\"> or this is a desktop layout at \
                         phone size",
                        env.inner_width, env.inner_height, vw, vh
                    );
                }
            }
        }
    }

    fn run_finished(&mut self, summary: Summary, elapsed: Duration) {
        if self.quiet {
            return;
        }
        self.clear_line();
        let total = summary.total();
        if self.tty && total > 0 {
            // Without this the bar's last drawn frame is the one from before
            // the final device, so a finished run visually reads as
            // unfinished. Draw the completed state explicitly.
            eprintln!(
                "[{}] {}/{}",
                progress::bar(total, total, self.width),
                total,
                total
            );
        }
        eprintln!(
            "\n{} exact, {} off-size, {} failed in {}",
            summary.exact,
            summary.off_size,
            summary.failed,
            progress::human(elapsed)
        );
    }
}

#[cfg(unix)]
fn is_tty() -> bool {
    // Avoiding a dependency for one libc call. STDERR_FILENO is 2.
    extern "C" {
        fn isatty(fd: i32) -> i32;
    }
    unsafe { isatty(2) == 1 }
}

#[cfg(not(unix))]
fn is_tty() -> bool {
    // Conservative on Windows: plain lines always render correctly, whereas
    // in-place redraws on a non-TTY produce unreadable logs.
    false
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

    let browser = cdp::find_browser(None).map_err(|e| e.to_string())?;

    let opts = RunOptions {
        url: url.to_string(),
        devices: targets,
        out_dir: out_dir.clone(),
        determinism: det,
        stability: Stability::default(),
        browser,
        fail_fast: args.has("fail-fast"),
    };

    let mut progress = TerminalProgress::new(args.has("quiet"));
    let report = run(&opts, &mut progress).map_err(|e| e.to_string())?;

    if args.has("json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        println!("{} captures -> {}", report.results.len(), out_dir.display());
        if report.off_size > 0 {
            println!("SOME CAPTURES ARE THE WRONG SIZE — do not upload these");
        }
        if report.failed > 0 {
            println!("{} device(s) failed to capture at all", report.failed);
        }
        if report.results.is_empty() {
            println!("nothing was captured");
        }
    }
    Ok(report.ok)
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
