// Smoke test for the Node binding, run against a real browser.
//
// This asserts the binding actually captures at the required pixel sizes and
// that its determinism matches the CLI's. A binding test that only checks
// "the module loads" tells you nothing about whether it works.

import assert from 'node:assert/strict'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const { capture, devices, version, findBrowser } = await import('../index.js')

const out = mkdtempSync(join(tmpdir(), 'proofsheet-node-'))
const page = join(out, 'subject.html')
writeFileSync(
  page,
  `<!doctype html><meta charset=utf-8>
   <style>body{margin:0;min-height:100vh;background:#0E3B37;color:#F7F5F2;
   font:16px system-ui;display:grid;place-items:center}</style>
   <div><p id=v></p><p id=t></p><p id=r></p></div>
   <script>
     v.textContent = innerWidth+'x'+innerHeight+'@'+devicePixelRatio;
     t.textContent = new Date().toISOString();
     r.textContent = Math.random();
   </script>`,
)

console.log('proofsheet native version:', version())
console.log('browser:', findBrowser())

// --- device table -----------------------------------------------------
const apple = devices('apple')
assert.ok(apple.length > 0, 'apple presets should not be empty')
for (const d of apple) {
  assert.equal(
    d.output_width % d.scale,
    0,
    `${d.id}: width does not divide by scale`,
  )
  assert.equal(d.output_height % d.scale, 0, `${d.id}: height does not divide by scale`)
  assert.ok(d.verified, `${d.id}: store preset must be verified`)
}
console.log(`devices(): ${apple.length} apple presets, all divisible + verified`)

assert.throws(() => devices('nonsense'), /unknown store/, 'bad store must throw')

// --- capture ----------------------------------------------------------
const ids = ['apple-iphone-6-9-1320', 'apple-ipad-13-2064', 'play-feature-graphic']
const a = await capture({ url: `file://${page}`, devices: ids, outDir: join(out, 'a'), seed: 42 })

assert.equal(a.results.length, ids.length)
assert.ok(a.ok, `run should succeed: ${JSON.stringify(a.results, null, 2)}`)
for (const r of a.results) {
  assert.equal(r.outcome, 'exact', `${r.deviceId} was ${r.outcome}: ${r.error ?? ''}`)
  const c = r.capture
  assert.ok(c, `${r.deviceId} produced no capture`)
  assert.equal(c.actualWidth, c.expectedWidth, `${r.deviceId} wrong width`)
  assert.equal(c.actualHeight, c.expectedHeight, `${r.deviceId} wrong height`)
  assert.ok(c.exact)
  console.log(
    `  ${r.deviceId.padEnd(24)} ${`${c.actualWidth}x${c.actualHeight}`.padStart(11)}  ${c.sha256.slice(0, 16)}`,
  )
}

// --- determinism, both directions ------------------------------------
const b = await capture({ url: `file://${page}`, devices: ids, outDir: join(out, 'b'), seed: 42 })
for (let i = 0; i < ids.length; i++) {
  assert.equal(
    a.results[i].capture.sha256,
    b.results[i].capture.sha256,
    `${ids[i]}: same seed produced different bytes`,
  )
}
console.log('same seed -> byte-identical: ok')

const c = await capture({ url: `file://${page}`, devices: [ids[0]], outDir: join(out, 'c'), seed: 999 })
assert.notEqual(
  a.results[0].capture.sha256,
  c.results[0].capture.sha256,
  'a different seed produced identical bytes, so the determinism layer is inert',
)
console.log('different seed -> differs: ok')

// --- the ok flag is not merely failed===0 ----------------------------
assert.equal(a.ok, true)
assert.equal(a.exact, ids.length)
assert.equal(a.offSize, 0)
assert.equal(a.failed, 0)
assert.equal(typeof a.elapsedMs, 'number')
assert.equal(a.proofsheet, version(), 'report must name the core that produced it')

// --- errors are loud --------------------------------------------------
await assert.rejects(
  capture({ url: `file://${page}`, devices: ['no-such-device'], outDir: out }),
  /unknown device id/,
  'an unknown device id must reject, not silently capture fewer',
)
await assert.rejects(
  capture({ url: `file://${page}`, outDir: out }),
  /specify either/,
  'omitting both devices and store must reject',
)
console.log('error paths: ok')

console.log('\nnode binding smoke test PASSED')
