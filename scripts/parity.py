#!/usr/bin/env python3
"""Prove the CLI, the Node binding and the Python binding agree byte for byte.

Three surfaces wrap one core. That is only true if it is checked: a binding
that quietly builds different RunOptions produces plausible images at the
wrong metrics, and nothing about the output looks wrong.

So: capture the same page, same devices, same seed, through all three, and
require identical SHA-256 digests. Any divergence is a real defect in one of
the three, and this is the only test that can see it.

Usage:
    parity.py <cli-binary> <node-package-dir> <url> [device ...]
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile

DEFAULT_DEVICES = [
    "apple-iphone-6-9-1320",
    "apple-ipad-13-2064",
    "play-feature-graphic",
]


def run(cmd, **kw):
    p = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if p.returncode != 0:
        print(f"command failed: {' '.join(map(str, cmd))}")
        print(p.stdout[-3000:])
        print(p.stderr[-3000:])
        raise SystemExit(1)
    return p.stdout


def via_cli(binary, url, devices, out):
    args = [binary, "capture", "--url", url, "--out", out, "--json", "--quiet"]
    for d in devices:
        args += ["--device", d]
    report = json.loads(run(args))
    return {
        r["device_id"]: r["capture"]["sha256"]
        for r in report["results"]
        if r.get("capture")
    }


def via_node(pkg_dir, url, devices, out):
    script = f"""
const {{ capture }} = require({json.dumps(os.path.join(pkg_dir, "index.js"))});
capture({{
  url: {json.dumps(url)},
  devices: {json.dumps(devices)},
  outDir: {json.dumps(out)},
  seed: 42,
}}).then((r) => {{
  const m = {{}};
  for (const x of r.results) if (x.capture) m[x.deviceId] = x.capture.sha256;
  process.stdout.write(JSON.stringify(m));
}}).catch((e) => {{ console.error(e); process.exit(1); }});
"""
    return json.loads(run(["node", "-e", script]))


def via_python(url, devices, out):
    import proofsheet

    rep = proofsheet.capture(url, out_dir=out, device_ids=devices, seed=42)
    return {
        r.device_id: r.capture.sha256 for r in rep.results if r.capture is not None
    }


def main() -> int:
    if len(sys.argv) < 4:
        print(__doc__)
        return 2
    cli, node_pkg, url = sys.argv[1], sys.argv[2], sys.argv[3]
    devices = sys.argv[4:] or DEFAULT_DEVICES

    tmp = tempfile.mkdtemp(prefix="proofsheet-parity-")
    results = {
        "cli": via_cli(cli, url, devices, os.path.join(tmp, "cli")),
        "node": via_node(node_pkg, url, devices, os.path.join(tmp, "node")),
        "python": via_python(url, devices, os.path.join(tmp, "py")),
    }

    for name, digests in results.items():
        if not digests:
            print(f"FAIL: {name} produced no digests, so this proves nothing")
            return 1
        if set(digests) != set(devices):
            print(f"FAIL: {name} captured {sorted(digests)}, wanted {sorted(devices)}")
            return 1

    print(f"{'device':<28}{'cli':<18}{'node':<18}{'python':<18}agree")
    ok = True
    for d in devices:
        row = [results[s][d] for s in ("cli", "node", "python")]
        agree = len(set(row)) == 1
        ok &= agree
        print(
            f"{d:<28}"
            + "".join(h[:16].ljust(18) for h in row)
            + ("yes" if agree else "NO")
        )

    print()
    if ok:
        print("PARITY: all three surfaces produced byte-identical images")
        return 0
    print("PARITY FAILED: the surfaces disagree, so one of them is wrong")
    return 1


if __name__ == "__main__":
    sys.exit(main())
