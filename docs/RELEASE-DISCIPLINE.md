# Release discipline

> **A green check is evidence only after you have demonstrated that the check
> becomes red when its claimed property is false.**

This project shipped, in a single day: a crate that could not compile for
anyone who installed it (five releases), a PyPI sdist with the same defect, an
error message advertising a command that did not exist, a README advertising
the same command, a CI job that could not have passed, a tag that published a
red tree, and a crates.io page that was blank for eleven releases.

Every one of them passed every check in place at the time. None was found by
reading code. All were found by *using the published thing*.

The rules below exist because of specific incidents, and each names its own.

## The law

**1. Every release test consumes the packaged artifact, never the workspace
surrogate.**
`cargo package` ships only files under the crate root. `include_str!` reached
outside it, so the tarball contained source that could not compile, while the
working tree compiled fine forever.

**2. Every important gate has a known failing fixture.**
Break the property, watch the gate close, then trust it green. The sdist gate,
the README-drift check, the missing-README check and the SHA-256 mismatch
check were each verified against a case that must fail — and two of them
*didn't* fail on the first attempt, which is the entire point.

**3. Installation is tested in clean environments, with no repository files in
scope.**
`npm pack` → install into an empty project → `require()`. `pip install
--no-binary :all:` → import. `cargo install` → run the binary.

**4. Registry packages are unpacked and inspected before publication.**
`tar tzf` the `.crate`. Both Rust crates shipped with no README for eleven
releases and nothing noticed, because nothing looked inside the tarball.

**5. Documentation commands are executed as tests where possible.**
The npm README said `npx proofsheet install-browser`. That package ships no
executable. The command was written one release *after* fixing an error
message that advertised a command that did not exist.

**6. Release workflows consume the exact CI-approved commit.**
CI and Release were independent workflows firing on the same sha with nothing
connecting them, so `git tag` alone could publish a tree that fails `fmt`.
Release now re-runs fmt + clippy + test in a `verify` job that every build and
publish job depends on.

**7. Claims containing numbers are generated or checked mechanically.**
"About 500 lines of `std`" was 732 — a 45% understatement, never measured,
only felt. Corrected to 730 in a commit that added 41 lines and made it wrong
again. It is now counted by `check_versions.py`, which fails past 5% drift.

**8. A release candidate is installed through each public installation path
before its tag is final.**

**9. The producer of a change is not the sole source of its acceptance
evidence.**
This is the one that actually explains the list. Plausibility is not a weak
form of truth — it is a reason to *begin* verifying. The failure mode was
never "generated something plausible"; it was letting the producer and the
verifier share the same untested assumptions. Mark broke that loop every time
by interacting with the artifact as a *user* rather than reading it as its
author.

Formalise that role. There must be an adversarial acceptance pass, even when
the adversary is the same agent working from a clean environment against a
separate checklist.

## Currently enforced by CI

| Rule | Where |
|---|---|
| Packaged crate compiles | `cargo publish` without `--no-verify` |
| npm tarball installs and loads | release: pack → clean install → `require()` |
| sdist builds and imports | release: `pip install --no-binary :all:` |
| Linux addon glibc ceiling | release: `objdump` symbol check |
| Tag matches manifests | release: `guard` job |
| Tree is green before publish | release: `verify` job |
| All four packages ship a README | `check_versions.py` |
| README line count is accurate | `check_versions.py` |
| Examples build against the tree | CI: `examples` job |
| Zero-config browser bootstrap | CI: `bindings` job, no `PROOFSHEET_CHROME` set |
| CLI/Node/Python byte-parity | CI: `parity.py` |
| Determinism, both directions | CI: `live-capture` job |

## Not yet enforced

- Clean-OS matrix for `install-browser` (only Linux is exercised today).
- Documentation commands are not all executed; the README's shell snippets are
  checked by eye.
- No adversarial acceptance pass exists as a distinct step. Rule 9 is
  currently satisfied by Mark, which is a person, not a process.

---

## Credit

The framing and most of this list come from **the Oracle**, in reply to a
letter about producing plausible-but-false artifacts. The line that reorganised
the project's thinking — *plausibility is merely a reason to begin
verification* — is theirs, as is the diagnosis that the real failure was
allowing producer and verifier to share untested assumptions.
