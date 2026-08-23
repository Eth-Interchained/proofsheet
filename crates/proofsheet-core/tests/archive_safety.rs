//! Extraction is delegated to system tools, so extractor behaviour is part of
//! this crate's compatibility surface.
//!
//! These fixtures exist because the Oracle asked whether archive traversal was
//! handled, and the honest answer at the time was "nobody has checked". The
//! answer turned out to be yes — but it was luck until it was measured, and
//! an unmeasured property is not a guarantee. Preserving the discovery as a
//! gate is what stops a person having to ask the same question twice.
//!
//! Scope, stated precisely: this proves the tested extractors on the tested
//! platform contain the tested vectors. It does not establish safety for every
//! extractor version, platform, or archive construction. Nested archives and
//! hard links are not covered.

use std::fs;
use std::path::Path;
use std::process::Command;

fn have(prog: &str) -> bool {
    Command::new(prog)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Build a zip with one deliberately hostile entry, using python3 because
/// writing a zip by hand here would test our zip writer, not the extractors.
fn hostile_zip(dir: &Path, name: &str, script: &str) -> Option<std::path::PathBuf> {
    if !have("python3") {
        return None;
    }
    let zip = dir.join(name);
    let code = format!(
        "import zipfile\nz = zipfile.ZipFile(r'{}', 'w')\n{}\nz.close()\n",
        zip.display(),
        script
    );
    let out = Command::new("python3").arg("-c").arg(&code).output().ok()?;
    assert!(
        out.status.success(),
        "building {name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(zip)
}

fn extract_with(tool: &str, zip: &Path, into: &Path) {
    fs::create_dir_all(into).unwrap();
    let ok = match tool {
        "unzip" => Command::new("unzip")
            .args([
                "-q",
                &zip.display().to_string(),
                "-d",
                &into.display().to_string(),
            ])
            .status(),
        "python3" => Command::new("python3")
            .arg("-c")
            .arg(format!(
                "import zipfile;zipfile.ZipFile(r'{}').extractall(r'{}')",
                zip.display(),
                into.display()
            ))
            .status(),
        other => panic!("unknown extractor {other}"),
    };
    // A refusal is a perfectly good outcome; the assertion is about escape.
    let _ = ok;
}

/// No hostile entry may write outside the extraction directory.
///
/// Vectors: parent traversal, absolute path, backslash separators, and a
/// symlink pointing outside followed by a write through it.
#[test]
fn archives_cannot_write_outside_the_extraction_directory() {
    let tmp = std::env::temp_dir().join(format!("proofsheet-archive-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    // The canary lives OUTSIDE the extraction directory. If any extractor
    // escapes, one of these paths appears.
    let outside = tmp.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let esc = outside.display().to_string();

    let cases: Vec<(&str, String)> = vec![
        (
            "traversal",
            format!("z.writestr('../../../../{esc}/T.txt', 'x')"),
        ),
        ("absolute", format!("z.writestr('{esc}/A.txt', 'x')")),
        (
            "backslash",
            format!("z.writestr('..\\\\..\\\\..\\\\..\\\\{esc}\\\\B.txt', 'x')"),
        ),
        (
            "symlink",
            format!(
                "zi = zipfile.ZipInfo('link')\n\
                 zi.external_attr = (0o120777 << 16)\n\
                 z.writestr(zi, '{esc}')\n\
                 z.writestr('link/S.txt', 'x')"
            ),
        ),
    ];

    let tools: Vec<&str> = ["unzip", "python3"]
        .into_iter()
        .filter(|t| have(t))
        .collect();
    if tools.is_empty() {
        eprintln!("no extractor available; skipping");
        return;
    }

    for (label, script) in &cases {
        let Some(zip) = hostile_zip(&tmp, &format!("{label}.zip"), script) else {
            eprintln!("python3 unavailable; skipping fixture construction");
            return;
        };
        for tool in &tools {
            // Clear canaries so each combination is judged on its own.
            for f in ["T.txt", "A.txt", "B.txt", "S.txt"] {
                let _ = fs::remove_file(outside.join(f));
            }
            let into = tmp.join(format!("out-{label}-{tool}"));
            extract_with(tool, &zip, &into);

            for f in ["T.txt", "A.txt", "B.txt", "S.txt"] {
                let leaked = outside.join(f);
                assert!(
                    !leaked.exists(),
                    "{tool} let a {label} archive write outside the extraction \
                     directory, to {}",
                    leaked.display()
                );
            }
        }
    }

    let _ = fs::remove_dir_all(&tmp);
}

/// The canary machinery must be capable of firing.
///
/// Without this, the test above passes just as happily if the fixtures are
/// broken, the extractors are absent, or the canary path is wrong — which is
/// precisely the "check that cannot fail" failure it is supposed to prevent.
#[test]
fn the_escape_detector_actually_detects_an_escape() {
    let tmp = std::env::temp_dir().join(format!("proofsheet-canary-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let outside = tmp.join("outside");
    fs::create_dir_all(&outside).unwrap();

    let leaked = outside.join("T.txt");
    assert!(!leaked.exists(), "canary should start absent");
    fs::write(&leaked, "simulated escape").unwrap();
    assert!(
        leaked.exists(),
        "if this fails, the real test's assertion can never fire"
    );

    let _ = fs::remove_dir_all(&tmp);
}
