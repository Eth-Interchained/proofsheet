# Oracle — design

> Status: design, not implemented. This is the specification the next phase
> builds against.

## The problem, stated correctly

The obvious framing is "build something that decides whether a screenshot is
correct." That framing is wrong and produces a bad machine.

The right framing: **build something that decides whether the available
evidence justifies accepting the capture.**

The difference matters because correctness is not fully contained in the
pixels. A desktop-looking mobile layout might be a bug — or it might be the
design. A near-blank page might be a failed render — or an intentionally
minimal opening frame. No general algorithm separates those without importing
intent from somewhere: a specification, a baseline, assertions, or a person.

That is a genuine property of the problem, not a gap in the implementation.
The human is not contamination in the oracle. **The human supplies the missing
predicate.**

## Six kinds of valid

| | | proofsheet can |
|---|---|---|
| 1 | **Mechanically valid** — dimensions, format, colour space, file integrity | establish strongly |
| 2 | **Execution-valid** — intended URL loaded, fonts settled, images complete, no fatal errors | establish strongly |
| 3 | **Platform-valid** — emulation, DPR, touch, safe areas, orientation, UA, viewport behaviour match the device contract | establish strongly |
| 4 | **Structurally plausible** — no blank screen, consent wall, login redirect, catastrophic overflow, clipped content, desktop nav at phone width, unresolved loading state | gather useful evidence |
| 5 | **Intent-valid** — shows the state, content and composition the author wanted | assist only |
| 6 | **Human-acceptable** — somebody would willingly publish it | assist only |

Today proofsheet does 1–3 and a sliver of 4 (`viewport_honoured`). Oracle's job
is to do 4 properly, structure the evidence for 5 and 6, and never pretend it
has crossed into them by itself.

## Verdicts, not booleans

Oracle must not return `correct: true`. It returns an evidence-backed verdict:

```json
{
  "verdict": "review",
  "confidence": 0.91,
  "mechanical": "pass",
  "execution": "pass",
  "platform": "pass",
  "structural": "suspicious",
  "findings": [
    {
      "code": "DESKTOP_LAYOUT_LIKELY",
      "evidence": {
        "requested_css_width": 430,
        "observed_inner_width": 1120,
        "horizontal_overflow_px": 690,
        "touch_points": 5
      }
    }
  ]
}
```

Verdict space, sharpened — the distinction between the last two carries most
of the operational weight:

| verdict | means |
|---|---|
| `accept` | affirmative evidence satisfies the configured policy |
| `reject` | a hard invariant or explicit contract was violated |
| `review` | the evidence is valid, but **intent** is required |
| `indeterminate` | Oracle could not obtain **trustworthy evidence** |

A capture that might intentionally use desktop composition is `review`. A
capture whose telemetry failed, whose page never stabilised, or whose
environment Oracle cannot model is `indeterminate`. Therefore:

- rising `indeterminate` is an **operational defect in Oracle**
- rising `review` is an **automation-coverage gap**
- a false `accept` is a **correctness failure**
- a false `reject` is an **overreach failure**

**An honest oracle must be allowed to abstain.** Without `indeterminate`,
uncertainty gets laundered into authority — the same failure as a green check
never shown capable of turning red.

### Abstention is metered, not free

Unpriced abstention is epistemic bankruptcy: perfectly honest, perfectly
useless. An Oracle returning `indeterminate` on 60% of captures has relocated
100% of the judgement to the human while keeping the appearance of a system.

But do not optimise the raw abstention rate either — that pressures Oracle to
manufacture confidence. Measure two properties **together**:

- **Coverage** — the share of captures Oracle decides automatically
- **Selective risk** — the error rate *among those automatic decisions*

That is a risk–coverage curve. Increasing coverage is valuable only while
error stays within policy.

Cost ordering, with the weights themselves belonging to policy — a marketing
screenshot and a medical-device screenshot must not share a threshold:

```
false accept  >  false reject  >  review  >  correct decision
indeterminate = operational failure cost
```

High abstention is acceptable early **only if it is visible, measured, and
trending down**. It is scaffolding, not a permanent defence.

## Layer 1 — hard invariants

May reject automatically. Every rejection carries measurements, not a score.

- Incorrect physical dimensions
- Unexpected URL or redirect
- Browser error page
- Empty or near-empty render
- Required selector absent
- Document not ready
- Fonts or critical images failed
- JavaScript exception designated fatal
- Layout viewport inconsistent with the device contract
- Capture taken before the declared readiness condition
- Prohibited UI present (dev overlays, debug banners)

## Layer 2 — generic structural warnings

Trigger `review`, never `reject`. This layer says "this resembles a known
failure", not "I know your intended design".

- Horizontal overflow
- Tiny median text size
- Desktop navigation patterns at mobile widths
- Excessive unused canvas
- Content clipped outside the visual viewport
- Fixed overlays occupying too much of the frame
- Cookie banners, permission dialogs
- Skeletons, spinners, broken-image glyphs, placeholder text
- Large visual change between repeated captures
- Primary content pushed below the fold
- Unexpected scroll position
- Low contrast / likely-unreadable text

## Layer 3 — counterfactual rendering

Don't inspect only the requested capture. Render controls and compare.

For a mobile target, also capture: the same pixel viewport with a **desktop
identity**, a **neighbouring mobile width**, and a **representative desktop
width**. Compare DOM geometry, breakpoints, element visibility, navigation
structure, text sizing, visual similarity.

Then Oracle can report something far stronger than "it looks desktop":

> This capture behaves more like the desktop control than like the mobile
> family.

Still evidence, not proof — but evidence with a comparison behind it. This
technique comes directly out of the wrong-layout bug that motivated the whole
feature.

## Layer 4 — declared intent

Optional per-project capture contract. The generic engine works without it;
intent-sensitive correctness gets stronger when the author states what matters.

```yaml
assert:
  visible:
    - "[data-testid=mobile-nav]"
    - "[data-testid=hero]"
  hidden:
    - "[data-testid=desktop-nav]"
  text:
    - "Create your account"
  max_horizontal_overflow: 0
  readiness: "window.__PROOFSHEET_READY__ === true"
```

Yes, assertions only catch anticipated failures. That is fine. They are
executable specifications, not a universal intelligence test.

## Layer 5 — human blessing and regression

A blessed baseline relocates judgement, and that is exactly what it should do.
The first acceptance is a **product** decision; every later comparison is an
**engineering** decision constrained by it.

Store with each accepted image: the capture recipe, browser and engine
versions, page evidence, a DOM/layout fingerprint, allowed regions of change,
and the reason and identity of the approver.

Do not reduce perceptual comparison to one pass/fail threshold. Produce an
overlay, changed regions, structural differences, and a recommendation — then
let a person decide whether the change was intended.

A vision model may act as *another reviewer* here, especially for obvious
desktop/mobile mismatch, broken composition, or dialogs. It must never be
presented as proof. Ask for findings tied to visible evidence, allow multiple
prompts or models, and permit disagreement.

## The resulting architecture

```
proofsheet captures
      ↓
Oracle assembles evidence
      ↓
Policy decides what may be rejected automatically
      ↓
Humans resolve intent
      ↓
Accepted judgements become regression knowledge
```

Worthy of the name because it does not pretend the uncertainty disappeared.

---

## Independence of acceptance evidence

The rule that explains all the others, in its corrected form:

> **Acceptance evidence must include at least one falsification source whose
> origin is independent of the implementation assumptions.**

Environmental cleanliness is not independence. A clean room removes shared
state; it does not remove shared assumptions. A checklist written by the
implementer verifies execution while remaining entirely inside the
implementer's model of the problem.

Qualifying sources:

- actual use by someone who did not implement the change
- a failure reported by a real consumer, preserved as a regression
- an external specification or conformance suite
- a failing fixture established **before** implementation
- a reviewer given the artifact and expected behaviour, but not the
  implementation's explanatory narrative
- mutation testing, fuzzing, or generated counterexamples the author did not
  individually select
- a differently prompted model whose task is to **falsify** the claim rather
  than understand or defend the implementation

"A different model" is not automatically independent. Two models given the
same narrative inherit the same framing and reproduce the same blind spot.
Independence is a property of the challenge's origin, not of who executes it.

### The scalable loop

A human at the boundary should not stay load-bearing for the same fact twice:

```
independent use discovers a failure class
      ↓
failure becomes a reproducible fixture
      ↓
fixture becomes an automated gate
      ↓
the human is released to find the next unknown class
```

Automation preserves each discovery. It never protects against the next
unknown assumption — that is what the human at the boundary is for.

## Challenger — a later component, deliberately not started

Oracle evaluates evidence. A **Challenger** would search for counterevidence:
it does not judge the artifact, it systematically generates ways the
artifact's claims could be false.

Feed it claim types, platform differences, historical incidents from
[RELEASE-DISCIPLINE.md](RELEASE-DISCIPLINE.md), attack taxonomies, mutation
operators, and the public installation paths. Keep its context **independent
of the implementation narrative** wherever possible — that independence is the
entire value, and sharing the narrative destroys it.

Measure it by **novel failing fixtures produced**, never by how convincing its
review reads. A Challenger that writes persuasive critiques and finds nothing
is the same failure as a green check that cannot turn red.

```
proofsheet  performs the work
Oracle      evaluates evidence
Challenger  searches for counterevidence
CI          preserves what reality taught them
humans      supply intent, and genuinely novel pressure
```

This is recorded, not scheduled. The honest reason: `docs/ORACLE.md` is
already 300 lines of design against ~40 lines of shipped behaviour change, and
Layer 1 does not exist yet. Writing a fourth component before implementing the
first would be the exact failure this document warns about — plausible
structure standing in for working machinery.

## Credit

This design is substantially the work of **the Oracle**, in reply to a letter
asking how to build an oracle that isn't a stability check wearing a costume.
The reframing — *evidence justifying acceptance* rather than *correctness* —
the six-layer validity ladder, the four-verdict space, counterfactual
rendering, and the blessing/regression split are all theirs. Recorded here
close to verbatim rather than paraphrased, because the precision is the value.
