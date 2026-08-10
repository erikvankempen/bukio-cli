#!/usr/bin/env node
/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */


// Runs the full bukio-cli test suite (each file in its own process, like
// `node --test` does) and regenerates test/report.md: every test with its
// pass/fail status, a one-line description per file, and the latest result.
// `npm test` = this script: exit code mirrors the suite, summary lines are
// printed for grepping, failures print the failing tests.
import { spawn } from 'node:child_process';
import { readdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import os from 'node:os';
import path from 'node:path';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const files = readdirSync(path.join(root, 'test')).filter((f) => f.endsWith('.test.js')).sort();

// Short description per test file — the suite's coverage map.
const DESCRIPTIONS = {
  'accounts.test.js': 'chart of accounts CRUD + CSV chart import',
  'actor.test.js': 'named-actor enforcement, actor identity CLI + sign-and-verify gate (record/enforce modes, stale/replay/registry refusals)',
  'actor-registry.test.js': 'per-company actor key registry: enrol/revoke (history kept), rotation, enforce flag, per-DB independence',
  'assets.test.js': 'fixed assets: schemes, mid-life adoption, runs, disposal, activastaat',
  'attachments.test.js': 'in-DB/file document attachments: add/list/show/remove, 25 MB cap, dedupe, metadata-only lists, audit',
  'audit.test.js': 'append-only audit log invariants',
  'backup.test.js': 'encrypted backups (AES-256-GCM), keep-N rotation, tamper detection, audited restore',
  'bank.test.js': 'CAMT.053/CSV import, idempotency, matching/reconciliation',
  'canonical.test.js': 'canonical command digest: stable sorted-key JSON, sha256, excludes identity/output flags, includes --dry-run',
  'cli.test.js': 'CLI end-to-end: init, entries, reports, backup/restore',
  'company.test.js': 'company show/update',
  'direct-debit.test.js': 'SEPA direct debit: mandate register, pain.008.001.02 export, FRST/RCUR, CORE/B2B split',
  'edge-cases.test.js': 'rounding, boundaries, idempotency, lifecycle violations, dry-run hygiene',
  'entries.test.js': 'journal entries: add/post/reverse, immutability',
  'export.test.js': 'export xaf (Auditfile 4.0, round-trips through the importer) + audit csv/xlsx',
  'import.test.js': 'opening balances, journal CSV, XAF (both layouts), contacts — whole-file validation, RGS inference',
  'import-invoice.test.js': 'inbound UBL (EN 16931/Peppol) invoice import into payables: idempotent, VAT reported not booked',
  'invoice.test.js': 'invoicing: lifecycle, 12-vereisten, credit notes, payments, reminders',
  'money.test.js': 'integer-cents money helpers',
  'month-end.test.js': 'month-end close check',
  'payments.test.js': 'SEPA payment batches: payables, pain.001 export',
  'agent-layer.test.js': 'MCP server, FX/ECB, tool gates, compliance calendar',
  'recurring-invoice.test.js': 'subscription invoice templates',
  'recurring.test.js': 'recurring entries engine: schedules, depreciation, accruals',
  'reports.test.js': 'balance sheet, P&L, journal',
  'reports-v014.test.js': 'aging buckets, contact statements, sales analytics (by contact/item)',
  'sign.test.js': 'ed25519 sign/verify/keyid module: keygen (plain + passphrase-encrypted PKCS8), roundtrip, tamper/wrong-key rejection',
  'smtp.test.js': 'zero-dependency SMTP client + invoice email: auth, STARTTLS, MIME/PDF attachment, dry-run, audit',
  'trial-balance.test.js': 'trial balance invariants',
  'vat.test.js': 'optional VAT module: codes, vat book, OB readout 1a–5d',
  'year-end.test.js': 'annual close, jaarrekening micro/klein, ICP',
};

function runFile(file) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, ['--test', '--test-reporter=tap', path.join('test', file)], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    child.stdout.on('data', (d) => { out += d; });
    child.stderr.on('data', (d) => { out += d; });
    child.on('close', (code) => {
      const tests = [];
      for (const line of out.split('\n')) {
        const m = line.match(/^(ok|not ok) \d+ - (.+)$/);
        if (m) tests.push({ name: m[2].trim(), ok: m[1] === 'ok' });
      }
      // a test file that crashed (syntax error, uncaught exception, runner
      // abort) exits non-zero with no TAP output — without this it silently
      // vanished from the suite and `npm test` reported green
      if (code !== 0 && tests.length === 0) {
        tests.push({ name: `file crashed — process exited ${code} (no test output)`, ok: false });
      }
      const pass = tests.filter((t) => t.ok).length;
      const fail = tests.filter((t) => !t.ok).length;
      resolve({ file, tests, pass, fail, exitCode: code });
    });
  });
}

// Run files with the same parallelism the test runner would use.
const concurrency = Math.max(1, os.availableParallelism?.() ?? 2);
const results = [];
const queue = [...files];
await Promise.all(Array.from({ length: Math.min(concurrency, queue.length) }, async () => {
  while (queue.length) {
    const file = queue.shift();
    results.push(await runFile(file));
  }
}));
results.sort((a, b) => a.file.localeCompare(b.file));

const pass = results.reduce((s, r) => s + r.pass, 0);
const fail = results.reduce((s, r) => s + r.fail, 0);
const tests = pass + fail;
const ok = fail === 0;
const ts = new Date().toISOString().replace('T', ' ').replace(/\.\d+Z$/, ' UTC');

const perFile = results.map((r) => {
  const list = r.tests.map((t) => `    - ${t.ok ? '✅' : '❌'} ${t.name}`).join('\n');
  return `### ${r.file} — ${DESCRIPTIONS[r.file] ?? ''}\n\n${r.pass} passing · ${r.fail} failing\n\n${list}`;
}).join('\n\n');

const report = `# bukio-cli — test report

**Latest run:** ${ts} — **${ok ? '✅' : '❌'} ${pass} passing · ${fail} failing (${tests} tests)**
**Command:** \`npm test\` (per-file \`node --test --test-reporter=tap\`)

## All tests

${perFile}

---
_Regenerated automatically on every \`npm test\`._
`;
writeFileSync(path.join(root, 'test', 'report.md'), report);

console.log(`# tests ${tests}`);
console.log(`# pass ${pass}`);
console.log(`# fail ${fail}`);
if (!ok) {
  for (const r of results) {
    for (const t of r.tests) if (!t.ok) console.log(`FAIL ${r.file}: ${t.name}`);
  }
  process.exit(1);
}
