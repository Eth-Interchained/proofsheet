#!/usr/bin/env node
// Capture a site at every App Store and Google Play size.
//
//   node shots.js <url> <out-dir>
//   node shots.js https://ourlynx.com ./ourlynx-shots
//
// Exits non-zero if anything is off-size or failed, so it drops straight
// into CI without extra plumbing.

const { capture, version, findBrowser } = require('@interchained/proofsheet');
const path = require('path');

const url = process.argv[2] || 'https://ourlynx.com';
const outDir = path.resolve(process.argv[3] || './shots');

// Both stores, written to their own subdirectory. Preset ids are already
// prefixed (apple-*, play-*) so a shared directory would not actually
// collide -- but store-per-folder is what you upload from.
const STORES = ['apple', 'play'];

async function main() {
  console.log(`proofsheet ${version()}`);
  console.log(`browser    ${findBrowser()}`);
  console.log(`url        ${url}`);
  console.log(`out        ${outDir}\n`);

  const reports = [];

  for (const store of STORES) {
    const dest = path.join(outDir, store);
    process.stdout.write(`${store.padEnd(6)} capturing... `);

    const report = await capture({
      url,
      store,
      outDir: dest,
      // Same seed both runs, so a re-run is byte-comparable against this one.
      seed: 42,
      locale: 'en-US',
      timezone: 'UTC',
    });

    reports.push({ store, report });
    console.log(
      `${report.exact} exact, ${report.offSize} off-size, ` +
      `${report.failed} failed  (${(report.elapsedMs / 1000).toFixed(1)}s)`
    );

    // A capture can be exactly the right number of pixels and still show the
    // wrong layout: if the page overrides the layout viewport, you get the
    // desktop skin scaled into a phone frame. Size alone will not catch it.
    const wrongLayout = report.results.filter(
      (r) => r.capture?.environment && !r.capture.environment.viewportHonoured
    );
    if (wrongLayout.length) {
      console.log(`       WARNING: ${wrongLayout.length} shot(s) got the wrong layout`);
      for (const r of wrongLayout.slice(0, 3)) {
        const e = r.capture.environment;
        console.log(`         ${r.deviceId}: page laid out at ${e.innerWidth}px CSS`);
      }
    }

    for (const r of report.results.filter((r) => r.outcome !== 'exact')) {
      const c = r.capture;
      const detail = c
        ? `wanted ${c.expectedWidth}x${c.expectedHeight}, got ${c.actualWidth}x${c.actualHeight}`
        : r.error;
      console.log(`       ${r.outcome.toUpperCase()} ${r.deviceId}: ${detail}`);
    }
  }

  const total = reports.reduce(
    (a, { report }) => ({
      exact: a.exact + report.exact,
      bad: a.bad + report.offSize + report.failed,
    }),
    { exact: 0, bad: 0 }
  );

  console.log(`\n${total.exact} exact, ${total.bad} problem(s) -> ${outDir}`);

  // report.ok, not failed === 0 -- the latter is also true for a run that
  // captured nothing at all.
  const allOk = reports.every(({ report }) => report.ok);
  process.exit(allOk ? 0 : 1);
}

main().catch((err) => {
  console.error(`\nfailed: ${err.message}`);
  process.exit(1);
});
