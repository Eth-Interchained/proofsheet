#!/usr/bin/env python3
"""Assert every version-bearing file agrees.

NEDB taught this the expensive way: version strings drift silently across
manifests, and the symptom is a published artifact whose reported version does
not match what is in it. Cheaper to fail a 200ms CI job.

Usage:
    check_versions.py            # verify
    check_versions.py 0.2.0      # rewrite all of them, then verify
"""
from __future__ import annotations

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# (path, human name, regex with a single capture group for the version)
SOURCES = [
    (
        "Cargo.toml",
        "workspace.package.version",
        re.compile(r'(?m)^\[workspace\.package\][^\[]*?^version\s*=\s*"([^"]+)"'),
    ),
    (
        "crates/proofsheet-py/pyproject.toml",
        "project.version",
        re.compile(r'(?m)^\[project\][^\[]*?^version\s*=\s*"([^"]+)"'),
    ),
]


def npm_version() -> str:
    p = ROOT / "crates/proofsheet-node/package.json"
    return json.loads(p.read_text())["version"]


def read_all() -> dict[str, str]:
    found: dict[str, str] = {}
    for rel, label, pattern in SOURCES:
        text = (ROOT / rel).read_text()
        m = pattern.search(text)
        if not m:
            raise SystemExit(f"could not find {label} in {rel}")
        found[f"{rel} ({label})"] = m.group(1)
    found["crates/proofsheet-node/package.json (version)"] = npm_version()
    return found


def bump(new: str) -> None:
    # npm requires strict semver; PEP 440 pre-release spellings differ from
    # semver's. Refuse anything that is not plain X.Y.Z rather than emitting
    # a version one registry will silently normalise and the other reject.
    if not re.fullmatch(r"\d+\.\d+\.\d+", new):
        raise SystemExit(
            f"refusing to set {new!r}: use plain X.Y.Z, which is valid "
            "unchanged on crates.io, npm and PyPI"
        )
    for rel, _label, pattern in SOURCES:
        path = ROOT / rel
        text = path.read_text()
        m = pattern.search(text)
        span = m.span(1)
        path.write_text(text[: span[0]] + new + text[span[1] :])
        print(f"  {rel} -> {new}")

    p = ROOT / "crates/proofsheet-node/package.json"
    data = json.loads(p.read_text())
    data["version"] = new
    p.write_text(json.dumps(data, indent=2) + "\n")
    print(f"  crates/proofsheet-node/package.json -> {new}")


# Development happens on a mirror; the advertised public repository is the
# one users are pointed at from a package page. Anything that ships must cite
# the public path, or npm and PyPI will send people to a repo that is not the
# published home of the project.
PUBLIC_REPO = "github.com/interchained/proofsheet"
DEV_REPO = "github.com/Eth-Interchained/proofsheet"

SHIPPED = [
    "Cargo.toml",
    "crates/proofsheet-node/package.json",
    "crates/proofsheet-node/README.md",
    "crates/proofsheet-py/pyproject.toml",
    "crates/proofsheet-py/README.md",
    "README.md",
]


CONTACT = "dev@interchained.org"
SITE = "interchained.org"
ATTRIBUTION = "Interchained LLC Labs"

# Personal/legacy addresses that must never reach a public package page.
FORBIDDEN = ["vibecode-101.com"]

MANIFESTS = [
    "Cargo.toml",
    "crates/proofsheet-node/package.json",
    "crates/proofsheet-py/pyproject.toml",
]

READMES = [
    "README.md",
    "crates/proofsheet-node/README.md",
    "crates/proofsheet-py/README.md",
]


def check_metadata() -> list[str]:
    """Everything that ships must cite the public repo, site and contact.

    These are the strings a stranger sees on npm and PyPI. Getting one wrong
    is not a cosmetic error -- it sends people somewhere that is not the
    published home of the project, or leaks a personal address.
    """
    problems = []

    for rel in SHIPPED:
        path = ROOT / rel
        if not path.exists():
            problems.append(f"{rel}: missing")
            continue
        text = path.read_text()
        for n, line in enumerate(text.splitlines(), 1):
            if DEV_REPO in line:
                problems.append(
                    f"{rel}:{n} points at the dev mirror; use {PUBLIC_REPO}"
                )
            for bad in FORBIDDEN:
                if bad in line:
                    problems.append(
                        f"{rel}:{n} contains {bad}; public contact is {CONTACT}"
                    )

    # Presence, not merely absence of the wrong thing.
    for rel in MANIFESTS:
        text = (ROOT / rel).read_text()
        if PUBLIC_REPO not in text:
            problems.append(f"{rel}: does not cite {PUBLIC_REPO}")
        if CONTACT not in text:
            problems.append(f"{rel}: does not carry the contact {CONTACT}")
        if SITE not in text:
            problems.append(f"{rel}: does not link {SITE}")
        if ATTRIBUTION not in text:
            problems.append(f"{rel}: does not credit {ATTRIBUTION}")

    for rel in READMES:
        text = (ROOT / rel).read_text()
        if CONTACT not in text:
            problems.append(f"{rel}: no contact address")
        if ATTRIBUTION not in text:
            problems.append(f"{rel}: no {ATTRIBUTION} attribution")

    return problems


def main() -> int:
    if len(sys.argv) > 1:
        print(f"bumping to {sys.argv[1]}")
        bump(sys.argv[1])

    meta = check_metadata()
    if meta:
        print("REPOSITORY METADATA PROBLEMS:")
        for p in meta:
            print("  -", p)
        return 1
    print(f"repository metadata: all shipped files cite {PUBLIC_REPO}\n")

    found = read_all()
    width = max(len(k) for k in found)
    for k, v in found.items():
        print(f"{k:<{width}}  {v}")

    distinct = set(found.values())
    if len(distinct) != 1:
        print(f"\nVERSION DRIFT: {sorted(distinct)}")
        print("Every manifest must carry the same version. Run:")
        print(f"  python3 scripts/check_versions.py <version>")
        return 1
    check_readme_line_count()

    print(f"\nall manifests agree: {distinct.pop()}")
    return 0


def check_readme_line_count() -> None:
    """The README quotes a line count for the hand-rolled client.

    It has been wrong twice: first "about 500" when it was 732, then "about
    730" written in the same commit that added 41 lines to ws.rs. A number a
    human maintains by feel is a number that drifts, so measure it.
    """
    import re

    root = pathlib.Path(__file__).resolve().parent.parent
    actual = sum(
        len((root / "crates/proofsheet-core/src" / f).read_text().splitlines())
        for f in ("ws.rs", "cdp.rs")
    )
    text = (root / "README.md").read_text()
    m = re.search(r"about (\d+) lines of `std`", text)
    if not m:
        raise SystemExit("README no longer states a line count; update this check")
    claimed = int(m.group(1))
    # Round numbers are fine; being off by more than 5% is not.
    if abs(claimed - actual) > actual * 0.05:
        raise SystemExit(
            f"README claims ~{claimed} lines of std for ws.rs + cdp.rs, "
            f"actual is {actual}. Fix the README."
        )
    print(f"README line count: claims ~{claimed}, actual {actual} — within 5%")


if __name__ == "__main__":
    sys.exit(main())
