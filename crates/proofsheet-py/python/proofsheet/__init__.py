"""proofsheet — exact-pixel store screenshots from a real browser.

Deterministic and local-first. Nothing is uploaded, no service is contacted,
and the model (when the agent features land) is whichever one you point it at.

    import proofsheet

    report = proofsheet.capture(
        "https://your.app",
        out_dir="./shots",
        store="apple",
    )
    if not report.ok:
        raise SystemExit(report.summary())

Why the sizes are exact: stores publish requirements in *output pixels*, while
browsers are driven in CSS pixels times a device pixel ratio. proofsheet stores
the required output size and derives the viewport, so a preset that could not
produce a required size is rejected before a browser ever starts.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Iterable, Optional, Sequence

from . import _native

__all__ = [
    "Capture",
    "DeviceResult",
    "Report",
    "Device",
    "capture",
    "devices",
    "find_browser",
    "__version__",
]

__version__: str = _native.version()


@dataclass(frozen=True)
class Device:
    """A capture target, as published by a store."""

    id: str
    label: str
    output_width: int
    output_height: int
    scale: int
    store: str
    requirement: str
    mobile: bool = False
    verified: bool = False
    source: str = ""

    @property
    def viewport(self) -> tuple[int, int]:
        """CSS-pixel viewport used to produce ``output_size``."""
        return (self.output_width // self.scale, self.output_height // self.scale)

    @property
    def output_size(self) -> tuple[int, int]:
        """Pixel dimensions the store requires."""
        return (self.output_width, self.output_height)

    @property
    def mandatory(self) -> bool:
        """Whether omitting this asset can block or degrade a submission."""
        return self.requirement not in ("optional", "recommended")


@dataclass(frozen=True)
class Capture:
    """One produced image."""

    expected: tuple[int, int]
    actual: tuple[int, int]
    sha256: str
    bytes: int
    exact: bool


@dataclass(frozen=True)
class DeviceResult:
    """What happened for one device."""

    device_id: str
    outcome: str
    elapsed_ms: int
    capture: Optional[Capture] = None
    error: Optional[str] = None
    path: Optional[str] = None

    @property
    def succeeded(self) -> bool:
        return self.outcome == "exact"


@dataclass(frozen=True)
class Report:
    """The result of a capture run."""

    proofsheet: str
    url: str
    seed: int
    locale: str
    exact: int
    off_size: int
    failed: int
    elapsed_ms: int
    ok: bool
    results: list[DeviceResult] = field(default_factory=list)

    def problems(self) -> list[DeviceResult]:
        """Every result that is not an exact capture."""
        return [r for r in self.results if not r.succeeded]

    def summary(self) -> str:
        """A one-or-more line description suitable for raising or logging."""
        head = (
            f"{self.exact} exact, {self.off_size} off-size, "
            f"{self.failed} failed in {self.elapsed_ms}ms"
        )
        if self.ok:
            return head
        lines = [head]
        if not self.results:
            lines.append("  nothing was captured")
        for r in self.problems():
            if r.capture is not None:
                lines.append(
                    f"  {r.device_id}: wanted {r.capture.expected}, "
                    f"got {r.capture.actual}"
                )
            else:
                lines.append(f"  {r.device_id}: {r.error}")
        return "\n".join(lines)


def _device(d: dict) -> Device:
    return Device(
        id=d["id"],
        label=d["label"],
        output_width=d["output_width"],
        output_height=d["output_height"],
        scale=d["scale"],
        store=d.get("store", "web"),
        requirement=d["requirement"],
        mobile=d.get("mobile", False),
        verified=d.get("verified", False),
        source=d.get("source", ""),
    )


def _report(d: dict) -> Report:
    results = []
    for r in d["results"]:
        cap = r.get("capture")
        results.append(
            DeviceResult(
                device_id=r["device_id"],
                outcome=r["outcome"],
                elapsed_ms=r["elapsed_ms"],
                capture=(
                    Capture(
                        expected=tuple(cap["expected"]),
                        actual=tuple(cap["actual"]),
                        sha256=cap["sha256"],
                        bytes=cap["bytes"],
                        exact=cap["exact"],
                    )
                    if cap
                    else None
                ),
                error=r.get("error"),
                path=r.get("path"),
            )
        )
    return Report(
        proofsheet=d["proofsheet"],
        url=d["url"],
        seed=d["seed"],
        locale=d["locale"],
        exact=d["exact"],
        off_size=d["off_size"],
        failed=d["failed"],
        elapsed_ms=d["elapsed_ms"],
        ok=d["ok"],
        results=results,
    )


def devices(store: Optional[str] = None, presets: Optional[str] = None) -> list[Device]:
    """List device presets, optionally filtered to ``apple``, ``play`` or ``web``.

    Raises ``ValueError`` for an unrecognised store rather than returning an
    empty list, because an empty list reads as "no devices" instead of "you
    typed the store name wrong".
    """
    return [_device(d) for d in json.loads(_native.devices_json(store, presets))]


def capture(
    url: str,
    out_dir: str = "./proofsheet-out",
    device_ids: Optional[Sequence[str]] = None,
    store: Optional[str] = None,
    seed: int = 42,
    locale: Optional[str] = None,
    timezone: Optional[str] = None,
    browser: Optional[str] = None,
    fail_fast: bool = False,
    presets: Optional[str] = None,
) -> Report:
    """Capture ``url`` at every requested device size.

    Specify either ``device_ids`` or ``store``. An unknown device id raises
    rather than being skipped: a run that quietly captures fewer images than
    you asked for produces an incomplete upload that looks successful.

    The same ``seed`` reproduces byte-identical images, which is what makes a
    regenerated screenshot set diffable.

    Releases the GIL while the browser works.
    """
    if isinstance(device_ids, str):
        # A bare string is iterable, so this would otherwise be read as a list
        # of single characters and fail with a baffling error.
        raise TypeError("device_ids must be a sequence of ids, not a single string")
    ids: Optional[list[str]] = list(device_ids) if device_ids is not None else None
    raw = _native.capture_json(
        url,
        out_dir,
        ids,
        store,
        seed,
        locale,
        timezone,
        browser,
        fail_fast,
        presets,
    )
    return _report(json.loads(raw))


def find_browser() -> str:
    """Path to the browser proofsheet would use.

    Raises ``RuntimeError`` explaining how to provide one if none is found.
    """
    return _native.find_browser()
