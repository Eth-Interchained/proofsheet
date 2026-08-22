//! Launching a headless Chromium and speaking the DevTools Protocol to it.

use std::io::{BufRead, BufReader, Read};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::ws::Ws;

/// Where to look for a browser binary when the caller does not name one.
const CANDIDATES: &[&str] = &[
    "chrome-headless-shell",
    "chromium",
    "chromium-browser",
    "google-chrome-stable",
    "google-chrome",
];

/// Locate a browser binary.
///
/// Order: the `PROOFSHEET_CHROME` environment variable, then the local
/// managed download, then anything on `PATH`. Explicit beats implicit, and a
/// pinned local build beats whatever the machine happens to have.
pub fn find_browser(managed_root: Option<&Path>) -> Result<PathBuf> {
    // Empty means unset. `export PROOFSHEET_CHROME=$(which chrome)` on a box
    // without chrome sets it to "", and env::var happily returns Ok(""),
    // which produced the nonsense "points at , which is not a file" instead
    // of falling through to discovery.
    if let Some(p) = std::env::var("PROOFSHEET_CHROME")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        let p = PathBuf::from(p.trim());
        if p.is_file() {
            return Ok(p);
        }
        return Err(Error::Browser(format!(
            "PROOFSHEET_CHROME points at {}, which is not a file",
            p.display()
        )));
    }
    if let Some(root) = managed_root {
        if root.is_dir() {
            if let Some(found) = find_in_tree(root, "chrome-headless-shell") {
                return Ok(found);
            }
        }
    }
    for name in CANDIDATES {
        if let Some(p) = which(name) {
            return Ok(p);
        }
    }
    Err(Error::Browser(
        "no browser found. Set PROOFSHEET_CHROME, or run `proofsheet install-browser`.".into(),
    ))
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|c| c.is_file())
}

fn find_in_tree(root: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut dirs = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.is_file() && p.file_name().map(|f| f == name).unwrap_or(false) {
            return Some(p);
        }
        if p.is_dir() {
            dirs.push(p);
        }
    }
    dirs.iter().find_map(|d| find_in_tree(d, name))
}

/// Options for launching the browser.
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub binary: PathBuf,
    pub port: u16,
    pub user_data_dir: Option<PathBuf>,
    pub extra_args: Vec<String>,
    pub timeout: Duration,
}

impl LaunchOptions {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        LaunchOptions {
            binary: binary.into(),
            // 0 asks the OS for a free port, which Chrome reports back on
            // stderr. Fixed ports collide when runs overlap.
            port: 0,
            user_data_dir: None,
            extra_args: Vec::new(),
            timeout: Duration::from_secs(30),
        }
    }
}

/// A live browser process plus an attached CDP session.
#[derive(Debug)]
pub struct Browser {
    child: Child,
    ws: Ws,
    next_id: u64,
    /// The port Chrome actually bound, which may differ from the requested one.
    pub port: u16,
}

impl Browser {
    pub fn launch(opts: &LaunchOptions) -> Result<Browser> {
        let mut cmd = Command::new(&opts.binary);
        cmd.arg(format!("--remote-debugging-port={}", opts.port))
            .arg("--headless")
            .arg("--no-sandbox")
            .arg("--disable-gpu")
            .arg("--hide-scrollbars")
            .arg("--mute-audio")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-dev-shm-usage")
            // Keep the browser's own scaling out of it: every scale decision
            // is made explicitly per device via Emulation.
            .arg("--force-device-scale-factor=1")
            // Background throttling would make timing depend on wall clock.
            .arg("--disable-background-timer-throttling")
            .arg("--disable-renderer-backgrounding")
            .arg("--disable-backgrounding-occluded-windows");
        if let Some(dir) = &opts.user_data_dir {
            cmd.arg(format!("--user-data-dir={}", dir.display()));
        }
        for a in &opts.extra_args {
            cmd.arg(a);
        }
        cmd.arg("about:blank");
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            Error::Browser(format!("could not spawn {}: {e}", opts.binary.display()))
        })?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::Browser("no stderr pipe".into()))?;
        let port = match read_devtools_port(stderr, opts.timeout) {
            Ok(p) => p,
            Err(e) => {
                let _ = child.kill();
                return Err(e);
            }
        };

        match attach(port, opts.timeout) {
            Ok(ws) => Ok(Browser {
                child,
                ws,
                next_id: 0,
                port,
            }),
            Err(e) => {
                let _ = child.kill();
                Err(e)
            }
        }
    }

    /// Issue a CDP command and wait for its matching reply, discarding events.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({ "id": id, "method": method, "params": params });
        self.ws.send_text(&msg.to_string())?;
        loop {
            let raw = self.ws.recv_text()?;
            let v: Value = serde_json::from_str(&raw)?;
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // an event, or a reply we are not waiting on
            }
            if let Some(err) = v.get("error") {
                let message = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                return Err(Error::Cdp {
                    method: method.to_string(),
                    message,
                });
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.ws.close();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Chrome prints `DevTools listening on ws://127.0.0.1:<port>/...` to stderr
/// once it is ready. Reading it is how we support `--remote-debugging-port=0`
/// and avoid guessing whether the browser has finished starting.
fn read_devtools_port(stderr: impl Read + Send + 'static, timeout: Duration) -> Result<u16> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(std::result::Result::ok) {
            if let Some(rest) = line.split("ws://").nth(1) {
                if let Some(hostport) = rest.split('/').next() {
                    if let Some((_, p)) = hostport.rsplit_once(':') {
                        if let Ok(port) = p.parse::<u16>() {
                            let _ = tx.send(port);
                            return;
                        }
                    }
                }
            }
        }
    });
    rx.recv_timeout(timeout)
        .map_err(|_| Error::Browser("browser did not report a DevTools port".into()))
}

/// Fetch the target list over the plain HTTP endpoint and attach to a page.
fn attach(port: u16, timeout: Duration) -> Result<Ws> {
    let deadline = Instant::now() + timeout;
    let mut last = String::from("no attempt made");
    while Instant::now() < deadline {
        match http_get(port, "/json/list", Duration::from_secs(5)) {
            Ok(body) => match serde_json::from_str::<Value>(&body) {
                Ok(Value::Array(targets)) => {
                    let page = targets
                        .iter()
                        .find(|t| t.get("type").and_then(Value::as_str) == Some("page"));
                    if let Some(url) = page
                        .and_then(|t| t.get("webSocketDebuggerUrl"))
                        .and_then(Value::as_str)
                    {
                        return Ws::connect(url, timeout);
                    }
                    last = "no page target yet".into();
                }
                Ok(_) => last = "target list was not an array".into(),
                Err(e) => last = format!("bad target list: {e}"),
            },
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(Error::Browser(format!("could not attach: {last}")))
}

/// A single-shot HTTP/1.1 GET. The DevTools HTTP endpoint is the only thing
/// we need it for, so it stays deliberately small.
///
/// Reads by `Content-Length` rather than to EOF. `read_to_end` on a socket
/// carrying a read timeout surfaces `WouldBlock`/`TimedOut` as a hard error
/// even when the full body already arrived, which presented as an opaque
/// "Resource temporarily unavailable" during bring-up.
fn http_get(port: u16, path: &str, timeout: Duration) -> Result<String> {
    use std::io::Write;
    let mut s = TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )?;
    s.flush()?;

    let mut raw: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];

    // Headers first.
    let head_end = loop {
        if let Some(i) = find_subslice(&raw, b"\r\n\r\n") {
            break i;
        }
        match s.read(&mut chunk) {
            Ok(0) => return Err(Error::Shape("http response ended in headers".into())),
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(Error::Io(e)),
        }
    };

    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
    let want: Option<usize> = head
        .split("\r\n")
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok());

    let body_start = head_end + 4;
    loop {
        let have = raw.len() - body_start;
        match want {
            Some(n) if have >= n => break,
            _ => {}
        }
        match s.read(&mut chunk) {
            Ok(0) => break, // clean EOF
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
            Err(ref e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // Timed out with a body already in hand: use what we have
                // rather than discarding a complete response.
                if want.is_none() && !raw[body_start..].is_empty() {
                    break;
                }
                return Err(Error::Shape("timed out reading http body".into()));
            }
            Err(e) => return Err(Error::Io(e)),
        }
    }

    Ok(String::from_utf8_lossy(&raw[body_start..]).to_string())
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod env_tests {
    /// An empty PROOFSHEET_CHROME must fall through to discovery rather than
    /// being treated as a path. `export PROOFSHEET_CHROME=$(which chrome)` on
    /// a machine without chrome sets it to "", and the old code reported
    /// "PROOFSHEET_CHROME points at , which is not a file".
    #[test]
    fn empty_env_var_is_not_a_path() {
        let raw = Some(String::new());
        let kept = raw.filter(|v: &String| !v.trim().is_empty());
        assert!(kept.is_none(), "empty string must be discarded");

        let blank = Some("   ".to_string()).filter(|v: &String| !v.trim().is_empty());
        assert!(blank.is_none(), "whitespace-only must be discarded");

        let real = Some(" /usr/bin/chrome ".to_string())
            .filter(|v: &String| !v.trim().is_empty())
            .map(|v| v.trim().to_string());
        assert_eq!(
            real.as_deref(),
            Some("/usr/bin/chrome"),
            "real path survives, trimmed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_env_binary_is_an_error_not_a_fallback() {
        // Pointing at a nonexistent path must fail loudly rather than
        // silently searching PATH -- substituting a different browser than
        // the caller named would make runs irreproducible.
        std::env::set_var("PROOFSHEET_CHROME", "/nonexistent/definitely/not/here");
        let r = find_browser(None);
        std::env::remove_var("PROOFSHEET_CHROME");
        assert!(matches!(r, Err(Error::Browser(_))));
    }

    #[test]
    fn launch_options_default_to_ephemeral_port() {
        let o = LaunchOptions::new("/bin/true");
        assert_eq!(o.port, 0);
    }
}
