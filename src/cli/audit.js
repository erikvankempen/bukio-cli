/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio audit — append-only audit log reader (JSON | csv | xlsx | human)
// and trail verifier (audit verify).
import { writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { list, verifyTrail } from '../audit/index.js';
import { toCsv, writeXlsx } from '../report/export.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

function auditColumns() {
  return [
    { key: 'id', label: 'id' },
    { key: 'ts', label: 'timestamp' },
    { key: 'actor', label: 'actor' },
    { key: 'action', label: 'action' },
    { key: 'command', label: 'command' },
    { key: 'args', label: 'args' },
    { key: 'outcome', label: 'outcome' },
    { key: 'entry_ids', label: 'entry_ids' },
  ];
}

function auditRow(r) {
  return {
    id: r.id, ts: r.ts, actor: r.actor, action: r.action, command: r.command,
    args: r.args ? JSON.stringify(r.args) : '',
    outcome: r.outcome,
    entry_ids: r.entry_ids?.length ? JSON.stringify(r.entry_ids) : '',
  };
}

export function make(program) {
  const audit = program
    .command('audit')
    .description('read the append-only audit log')
    .option('--since <iso-ts>', 'only entries at or after this timestamp')
    .option('--by <who>', 'only entries by this actor')
    .option('--limit <n>', 'max rows', '50')
    .option('--format <format>', 'json|csv|xlsx|human (default: json with --json, else human)')
    .option('--out <path>', 'output file (csv/xlsx)');
  audit
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const rows = list(db, {
            since: opts.since || null,
            actor: opts.by || null,
            limit: Number(opts.limit),
          });
          const format = opts.format || (ctx.json ? 'json' : 'human');
          if (format === 'json') {
            // --format json must print JSON even without the global --json
            // (output() with an empty render would print nothing)
            console.log(JSON.stringify({ ok: true, data: { entries: rows } }, null, 2));
            return;
          }
          if (format === 'csv') {
            const csv = toCsv(rows.map(auditRow), auditColumns());
            if (opts.out) {
              mkdirSync(path.dirname(path.resolve(opts.out)), { recursive: true });
              writeFileSync(opts.out, csv);
              console.log(`wrote ${opts.out}`);
            } else {
              process.stdout.write(csv);
            }
            return;
          }
          if (format === 'xlsx') {
            if (!opts.out) {
              const e = new Error('--out <path> is required for xlsx output');
              e.code = 'OUT_REQUIRED';
              throw e;
            }
            mkdirSync(path.dirname(path.resolve(opts.out)), { recursive: true });
            await writeXlsx(opts.out, [{
              name: 'Audit log',
              columns: auditColumns().map((c) => ({ header: c.label, key: c.key })),
              rows: rows.map(auditRow),
            }]);
            console.log(`wrote ${opts.out}`);
            return;
          }
          output(ctx, { entries: rows }, (d) => {
            table(d.entries, [
              { key: 'id', label: '#' },
              { key: 'ts', label: 'timestamp' },
              { key: 'actor', label: 'actor' },
              { key: 'action', label: 'action' },
              { key: 'command', label: 'command' },
              { key: 'outcome', label: 'outcome' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  audit
    .command('verify')
    .description('re-verify the signed audit trail against the company registry (exit 1 when tampered/invalid/unknown-key rows exist)')
    .option('--since <iso-ts>', 'only rows at or after this timestamp')
    .option('--limit <n>', 'check only the newest N rows')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const { rows, summary } = verifyTrail(db, {
            since: opts.since || null,
            limit: opts.limit ? Number(opts.limit) : null,
          });
          output(ctx, { summary, rows }, (d) => {
            console.log(
              `audit verify: ${d.summary.total} rows — ${d.summary.ok} ok, ${d.summary.unsigned} unsigned, `
              + `${d.summary.revoked} revoked, ${d.summary.tampered} tampered, `
              + `${d.summary.invalid_signature} invalid signature, ${d.summary.unknown_key} unknown key`,
            );
            const problems = d.rows.filter((r) => !['ok', 'unsigned'].includes(r.status));
            if (problems.length) {
              console.log('problem rows:');
              table(problems, [
                { key: 'id', label: '#' },
                { key: 'ts', label: 'timestamp' },
                { key: 'actor', label: 'actor' },
                { key: 'action', label: 'action' },
                { key: 'status', label: 'status' },
              ]);
            }
          });
          const bad = summary.tampered + summary.invalid_signature + summary.unknown_key;
          if (bad > 0) process.exitCode = 1;
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
