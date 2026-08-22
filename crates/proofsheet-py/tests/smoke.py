"""Smoke test for the Python binding, run against a real browser.

Asserts the binding actually captures at required pixel sizes and that its
determinism matches the CLI's. A binding test that only checks "the module
imports" tells you nothing about whether it works.
"""

import os
import tempfile

import proofsheet

out = tempfile.mkdtemp(prefix="proofsheet-py-")
page = os.path.join(out, "subject.html")
with open(page, "w") as f:
    f.write(
        """<!doctype html><meta charset=utf-8>
<style>body{margin:0;min-height:100vh;background:#0E3B37;color:#F7F5F2;
font:16px system-ui;display:grid;place-items:center}</style>
<div><p id=v></p><p id=t></p><p id=r></p></div>
<script>
  v.textContent = innerWidth+'x'+innerHeight+'@'+devicePixelRatio;
  t.textContent = new Date().toISOString();
  r.textContent = Math.random();
</script>"""
    )
url = "file://" + page

print("proofsheet version:", proofsheet.__version__)
print("browser:", proofsheet.find_browser())

# --- device table -----------------------------------------------------
apple = proofsheet.devices("apple")
assert apple, "apple presets should not be empty"
for d in apple:
    vw, vh = d.viewport
    assert (vw * d.scale, vh * d.scale) == d.output_size, f"{d.id}: viewport math"
    assert d.verified, f"{d.id}: store preset must be verified"
    assert d.source.startswith("https://"), f"{d.id}: missing source"
print(f"devices(): {len(apple)} apple presets, viewport math and sourcing ok")

try:
    proofsheet.devices("nonsense")
    raise AssertionError("unknown store should raise")
except ValueError as e:
    assert "unknown store" in str(e)

# --- capture ----------------------------------------------------------
ids = ["apple-iphone-6-9-1320", "apple-ipad-13-2064", "play-feature-graphic"]
a = proofsheet.capture(url, out_dir=os.path.join(out, "a"), device_ids=ids, seed=42)

assert len(a.results) == len(ids)
assert a.ok, a.summary()
for r in a.results:
    assert r.succeeded, f"{r.device_id}: {r.outcome} {r.error}"
    c = r.capture
    assert c is not None and c.actual == c.expected, f"{r.device_id}: wrong size"
    print(f"  {r.device_id:<24} {'x'.join(map(str, c.actual)):>11}  {c.sha256[:16]}")

assert a.proofsheet == proofsheet.__version__
assert not a.problems()
print("summary:", a.summary())

# --- determinism, both directions ------------------------------------
b = proofsheet.capture(url, out_dir=os.path.join(out, "b"), device_ids=ids, seed=42)
for x, y in zip(a.results, b.results):
    assert x.capture.sha256 == y.capture.sha256, f"{x.device_id}: same seed differed"
print("same seed -> byte-identical: ok")

c = proofsheet.capture(url, out_dir=os.path.join(out, "c"), device_ids=ids[:1], seed=999)
assert a.results[0].capture.sha256 != c.results[0].capture.sha256, (
    "a different seed produced identical bytes, so the determinism layer is inert"
)
print("different seed -> differs: ok")

# --- errors are loud --------------------------------------------------
for bad, expected in [
    (dict(device_ids=["no-such-device"]), "unknown device id"),
    (dict(), "specify either"),
]:
    try:
        proofsheet.capture(url, out_dir=out, **bad)
        raise AssertionError(f"expected failure for {bad}")
    except ValueError as e:
        assert expected in str(e), f"wrong message: {e}"

# A bare string is iterable; without a guard it would be read as a list of
# single characters and fail with something baffling.
try:
    proofsheet.capture(url, out_dir=out, device_ids="apple-iphone-6-9-1320")
    raise AssertionError("a bare string should be rejected")
except TypeError as e:
    assert "not a single string" in str(e)
print("error paths: ok")

print("\npython binding smoke test PASSED")
