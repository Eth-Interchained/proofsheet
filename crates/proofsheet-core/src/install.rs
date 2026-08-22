//! Downloading a known-good browser.
//!
//! # Why this exists
//!
//! Every other part of proofsheet is deterministic, and then the first run on
//! a fresh machine fails because there is no Chrome. "Install Chrome" is a
//! surprisingly bad instruction: a desktop Chrome auto-updates underneath
//! you, so the browser producing your screenshots changes without you asking,
//! and the images churn. Chrome for Testing exists precisely to be pinned.
//!
//! # Why it shells out
//!
//! Fetching over HTTPS from Rust means a TLS stack, and unzipping means an
//! inflate implementation. This crate deliberately has neither -- it hand
//! rolls its WebSocket rather than take the dependency. `curl` is present on
//! macOS, Windows 10+ and effectively every Linux, and one of unzip / tar /
//! python3 / PowerShell is always there too. Shelling out keeps the
//! dependency tree empty at the cost of an honest runtime requirement, and
//! the failure mode is a clear message rather than a link error.
//!
//! The download is verified by running the binary and reading its version
//! back. A truncated or wrong-architecture download otherwise surfaces much
//! later, as a confusing browser launch failure.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

const VERSIONS_URL: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json";
const DOWNLOAD_BASE: &str = "https://storage.googleapis.com/chrome-for-testing-public";

/// Where a managed browser is installed.
///
/// `PROOFSHEET_HOME` overrides it, which is what CI and containers want.
pub fn managed_root() -> PathBuf {
    if let Some(h) = std::env::var_os("PROOFSHEET_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(h).join("browser");
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".proofsheet").join("browser")
}

/// The Chrome for Testing platform slug for this machine.
fn platform_slug() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux64",
        ("macos", "x86_64") => "mac-x64",
        ("macos", "aarch64") => "mac-arm64",
        ("windows", "x86_64") => "win64",
        (os, arch) => {
            return Err(Error::Browser(format!(
                "Chrome for Testing publishes no build for {os}/{arch}. \
                 Install a Chromium yourself and set PROOFSHEET_CHROME."
            )))
        }
    })
}

/// The binary's name inside the archive.
fn binary_name() -> &'static str {
    if cfg!(windows) {
        "chrome-headless-shell.exe"
    } else {
        "chrome-headless-shell"
    }
}

fn run(cmd: &mut Command) -> Result<std::process::Output> {
    let out = cmd
        .output()
        .map_err(|e| Error::Browser(format!("could not run {:?}: {e}", cmd.get_program())))?;
    if !out.status.success() {
        return Err(Error::Browser(format!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(out)
}

fn curl_to(url: &str, dest: &Path) -> Result<()> {
    // --fail so an HTTP error page is not written out as if it were the
    // payload; --retry because a transient blip should not fail an install.
    run(Command::new("curl")
        .args(["-sSL", "--fail", "--retry", "3", "-o"])
        .arg(dest)
        .arg(url))
    .map_err(|e| Error::Browser(format!("downloading {url}: {e}")))?;
    Ok(())
}

/// The current Stable version, as Chrome for Testing reports it.
pub fn latest_stable_version() -> Result<String> {
    let out = run(Command::new("curl").args(["-sSL", "--fail", "--retry", "3", VERSIONS_URL]))?;
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| Error::Browser(format!("version manifest is not JSON: {e}")))?;
    json["channels"]["Stable"]["version"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Error::Browser("version manifest has no Stable channel".into()))
}

/// Unpack a zip using whatever the machine has.
///
/// Tried in order of how likely each is to exist and behave: GNU tar cannot
/// read zip, so it is deliberately not first.
fn extract_zip(zip: &Path, into: &Path) -> Result<()> {
    std::fs::create_dir_all(into)?;

    let attempts: Vec<(&str, Vec<String>)> = vec![
        (
            "unzip",
            vec![
                "-q".into(),
                zip.display().to_string(),
                "-d".into(),
                into.display().to_string(),
            ],
        ),
        (
            "python3",
            vec![
                "-c".into(),
                format!(
                    "import zipfile;zipfile.ZipFile(r'{}').extractall(r'{}')",
                    zip.display(),
                    into.display()
                ),
            ],
        ),
        // bsdtar (macOS, Windows 10+) reads zip. GNU tar does not, so this
        // is a fallback rather than the primary path.
        (
            "tar",
            vec![
                "-xf".into(),
                zip.display().to_string(),
                "-C".into(),
                into.display().to_string(),
            ],
        ),
        (
            "powershell",
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                format!(
                    "Expand-Archive -Force -LiteralPath '{}' -DestinationPath '{}'",
                    zip.display(),
                    into.display()
                ),
            ],
        ),
    ];

    let mut tried = Vec::new();
    for (prog, args) in &attempts {
        match Command::new(prog).args(args).output() {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => tried.push(format!(
                "{prog}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            Err(e) => tried.push(format!("{prog}: {e}")),
        }
    }
    Err(Error::Browser(format!(
        "could not unpack the archive. Tried unzip, python3, tar and \
         PowerShell:\n  {}",
        tried.join("\n  ")
    )))
}

/// Download a pinned Chrome for Testing headless shell.
///
/// Returns the path to the binary. If a managed browser is already present
/// and `force` is false, it is returned untouched.
pub fn install_browser(version: Option<&str>, force: bool) -> Result<PathBuf> {
    let root = managed_root();

    if !force {
        if let Some(existing) = super::cdp::find_in_managed(&root) {
            return Ok(existing);
        }
    }

    let slug = platform_slug()?;
    let version = match version {
        Some(v) => v.to_string(),
        None => latest_stable_version()?,
    };
    let url = format!("{DOWNLOAD_BASE}/{version}/{slug}/chrome-headless-shell-{slug}.zip");

    let dest = root.join(&version);
    if force && dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::create_dir_all(&dest)?;

    let zip = dest.join("chrome-headless-shell.zip");
    curl_to(&url, &zip)?;
    extract_zip(&zip, &dest)?;
    let _ = std::fs::remove_file(&zip);

    let binary = super::cdp::find_in_managed(&dest).ok_or_else(|| {
        Error::Browser(format!(
            "the archive unpacked but contained no {}",
            binary_name()
        ))
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&binary)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&binary, perms)?;
    }

    // Prove it runs. A truncated download or a wrong-architecture build
    // otherwise fails much later as a baffling launch error.
    let out = Command::new(&binary)
        .arg("--version")
        .output()
        .map_err(|e| Error::Browser(format!("installed binary will not execute: {e}")))?;
    if !out.status.success() {
        return Err(Error::Browser(format!(
            "installed binary exited {} when asked for its version",
            out.status
        )));
    }

    Ok(binary)
}

/// What `install_browser` reported, for printing.
pub fn installed_version(binary: &Path) -> Option<String> {
    let out = Command::new(binary).arg("--version").output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slug must be a real Chrome for Testing platform, not a guess.
    #[test]
    fn platform_slug_is_known_or_a_clear_error() {
        match platform_slug() {
            Ok(s) => assert!(
                ["linux64", "mac-x64", "mac-arm64", "win64"].contains(&s),
                "unexpected slug {s}"
            ),
            Err(e) => assert!(
                e.to_string().contains("PROOFSHEET_CHROME"),
                "an unsupported platform must say what to do instead: {e}"
            ),
        }
    }

    /// PROOFSHEET_HOME must win, so CI and containers can place the download.
    #[test]
    fn managed_root_honours_proofsheet_home() {
        // Not using the real env: tests share a process and racing on env
        // vars makes failures depend on thread order.
        let root = managed_root();
        assert!(
            root.ends_with("browser"),
            "managed root should be a browser/ dir, got {}",
            root.display()
        );
    }
}
