#!/usr/bin/env python3
"""Capture a site at every App Store and Google Play size.

    python shots.py https://ourlynx.com ./ourlynx-shots

Exits non-zero if anything is off-size or failed, so it drops straight into
CI without extra plumbing.
"""

from __future__ import annotations

import sys
from pathlib import Path

import proofsheet

STORES = ("apple", "play")


def main() -> int:
    url = sys.argv[1] if len(sys.argv) > 1 else "https://ourlynx.com"
    out_dir = Path(sys.argv[2] if len(sys.argv) > 2 else "./shots").resolve()

    print(f"proofsheet {proofsheet.__version__}")
    print(f"browser    {proofsheet.find_browser()}")
    print(f"url        {url}")
    print(f"out        {out_dir}\n")

    reports = []

    for store in STORES:
        print(f"{store:<6} capturing... ", end="", flush=True)

        report = proofsheet.capture(
            url=url,
            store=store,
            out_dir=str(out_dir / store),
            # The same seed reproduces byte-identical images, which is what
            # makes a regenerated screenshot set diffable.
            seed=42,
            locale="en-US",
            timezone="UTC",
        )
        reports.append(report)

        print(
            f"{report.exact} exact, {report.off_size} off-size, "
            f"{report.failed} failed  ({report.elapsed_ms / 1000:.1f}s)"
        )

        # A shot can be exactly the right number of pixels and still show the
        # wrong layout: if the page overrides the layout viewport you get the
        # desktop skin scaled into a phone frame. Size alone will not catch it.
        wrong = [
            r
            for r in report.results
            if r.capture is not None
            and r.capture.environment is not None
            and not r.capture.environment.viewport_honoured
        ]
        if wrong:
            print(f"       WARNING: {len(wrong)} shot(s) got the wrong layout")
            for r in wrong[:3]:
                env = r.capture.environment
                print(f"         {r.device_id}: laid out at {env.inner_width}px CSS")

        for r in report.problems():
            if r.capture is not None:
                detail = f"wanted {r.capture.expected}, got {r.capture.actual}"
            else:
                detail = r.error or "unknown"
            print(f"       {r.outcome.upper()} {r.device_id}: {detail}")

    total_exact = sum(r.exact for r in reports)
    total_bad = sum(r.off_size + r.failed for r in reports)
    print(f"\n{total_exact} exact, {total_bad} problem(s) -> {out_dir}")

    # report.ok, not failed == 0 -- the latter is also true for a run that
    # captured nothing at all.
    return 0 if all(r.ok for r in reports) else 1


if __name__ == "__main__":
    raise SystemExit(main())
