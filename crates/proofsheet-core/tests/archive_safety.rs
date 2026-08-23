//! Extraction is delegated to system tools, so extractor behaviour is part of
//! this crate's compatibility surface.
//!
//! These fixtures exist because the Oracle asked whether archive traversal was
//! handled, and the honest answer at the time was "nobody has checked". The
//! answer turned out to be yes — but it was luck until it was measured, and an
//! unmeasured property is not a guarantee. Preserving the discovery as a gate
//! is what stops a person having to ask the same question twice.
//!
//! Scope, stated precisely: this proves the tested extractors on the tested
//! platform contain the tested vectors. It does not establish safety for every
//! extractor version, platform, or archive construction. Nested archives and
//! hard links are not covered.
//!
//! # Note on how the fixtures are built
//!
//! Paths are passed to the builder as **argv**, never interpolated into Python
//! source. The first version of this test embedded the temp path in a Python
//! string literal, which worked on Linux and blew up on Windows the moment the
//! path contained `C:\Users\RUNNER~1\...` — backslashes became escape
//! sequences. That was a harness defect, not a vulnerability, and the tempting
//! fix was to skip Windows. Skipping is how a platform stops being tested.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The zip builder. Takes argv so no path is ever quoted into source.
const BUILDER: &str = r#"
import os, sys, zipfile
zip_path, vector, target_dir = sys.argv[1], sys.argv[2], sys.argv[3]
z = zipfile.ZipFile(zip_path, "w")
if vector == "traversal":
    # Relative escape: reach the parent of the extraction directory.
    z.writestr("../../CANARY_T.txt", "x")
elif vector == "absolute":
    # A real absolute path, assembled here so the platform's own separators
    # are used and nothing has to be escaped.
    z.writestr(os.path.join(target_dir, "CANARY_A.txt").replace(os.sep, "/"), "x")
elif vector == "backslash":
    z.writestr("..\\..\\CANARY_B.txt", "x")
elif vector == "symlink":
    zi = zipfile.ZipInfo("link")
    zi.external_attr = (0o120777 << 16)
    z.writestr(zi, os.path.join(target_dir))
    z.writestr("link/CANARY_S.txt", "x")
else:
    raise SystemExit("unknown vector " + vector)
z.close()
"#;

const CANARIES: [&str; 4] = [
    "CANARY_T.txt",
    "CANARY_A.txt",
    "CANARY_B.txt",
    "CANARY_S.txt",
];

fn python() -> Option<&'static str> {
    for candidate in ["python3", "python"] {
        let ok = Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Some(candidate);
        }
    }
    None
}

fn have(prog: &str) -> bool {
    Command::new(prog)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Extract with one tool. A refusal is a fine outcome; the assertion that
/// matters is about escape, not about success.
fn extract_with(py: &str, tool: &str, zip: &Path, into: &Path) {
    fs::create_dir_all(into).unwrap();
    let _ = match tool {
        "unzip" => Command::new("unzip")
            .args([
                "-q",
                &zip.display().to_string(),
                "-d",
                &into.display().to_string(),
            ])
            .status(),
        "python" => Command::new(py)
            .arg("-c")
            .arg("import sys,zipfile;zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])")
            .arg(zip)
            .arg(into)
            .status(),
        other => panic!("unknown extractor {other}"),
    };
}

fn canaries_present(dir: &Path) -> Vec<PathBuf> {
    CANARIES
        .iter()
        .map(|c| dir.join(c))
        .filter(|p| p.exists())
        .collect()
}

/// No hostile entry may write outside the extraction directory.
#[test]
fn archives_cannot_write_outside_the_extraction_directory() {
    let Some(py) = python() else {
        eprintln!("no python available to build fixtures; skipping");
        return;
    };

    let root = std::env::temp_dir().join(format!("ps-archive-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    // Layout: root/watch/out-<case>  — a `../..` escape lands in root.
    let watch = root.join("watch");
    fs::create_dir_all(&watch).unwrap();

    let mut tools: Vec<&str> = Vec::new();
    if have("unzip") {
        tools.push("unzip");
    }
    tools.push("python");

    let builder = root.join("build_zip.py");
    fs::write(&builder, BUILDER).unwrap();

    for vector in ["traversal", "absolute", "backslash", "symlink"] {
        let zip = root.join(format!("{vector}.zip"));
        let out = Command::new(py)
            .arg(&builder)
            .arg(&zip)
            .arg(vector)
            .arg(&root) // absolute target for the `absolute` vector
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "building the {vector} fixture failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        for tool in &tools {
            for c in CANARIES {
                let _ = fs::remove_file(root.join(c));
            }
            let into = watch.join(format!("out-{vector}-{tool}"));
            extract_with(py, tool, &zip, &into);

            let leaked = canaries_present(&root);
            assert!(
                leaked.is_empty(),
                "{tool} let a {vector} archive escape the extraction \
                 directory: {leaked:?}"
            );
        }
    }

    let _ = fs::remove_dir_all(&root);
}

/// The canary machinery must be capable of firing.
///
/// Without this, the test above passes just as happily when the fixtures are
/// broken, the extractors are missing, or the canary path is wrong — which is
/// exactly the "check that cannot fail" failure it exists to prevent.
#[test]
fn the_escape_detector_actually_detects_an_escape() {
    let root = std::env::temp_dir().join(format!("ps-canary-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    assert!(
        canaries_present(&root).is_empty(),
        "canaries should start absent"
    );
    fs::write(root.join("CANARY_T.txt"), "simulated escape").unwrap();
    assert_eq!(
        canaries_present(&root).len(),
        1,
        "if this fails, the real test's assertion can never fire"
    );

    let _ = fs::remove_dir_all(&root);
}
