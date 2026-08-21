/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio attach — store source documents against invoices/entries.
// Default mode stores the file IN the database (travels with backups);
// --store file keeps the DB lean and stores a path (content-addressed copy
// in <db>-attachments/).
import { existsSync, writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import {
  addAttachment, listAttachments, getAttachment, removeAttachment,
} from '../core/attachments.js';
import { ensureDb, makeCtx, output, fail, dbError, table, withDb } from './util.js';

function fmtBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Resolve --invoice/--entry into {kind, id}; throws REF_REQUIRED. */
function resolveRef(opts) {
  const hasInvoice = opts.invoice !== undefined && opts.invoice !== null && opts.invoice !== '';
  const hasEntry = opts.entry !== undefined && opts.entry !== null && opts.entry !== '';
  if (hasInvoice === hasEntry) {
    throw dbError('REF_REQUIRED', 'pass exactly one of --invoice <id> or --entry <id>');
  }
  return hasInvoice ? { kind: 'invoice', id: Number(opts.invoice) } : { kind: 'entry', id: Number(opts.entry) };
}

export function make(program) {
  const attach = program.command('attach').description('document attachments (source documents stored in the DB by default)');

  attach
    .command('add')
    .description('attach a source document to an invoice or entry (house rule 8)')
    .option('--invoice <id>', 'invoice id')
    .option('--entry <id>', 'journal entry id')
    .requiredOption('--file <path>', 'document to store (pdf/jpg/png/xml/eml/...)')
    .option('--store <mode>', "storage mode: 'db' (default — travels with backups) or 'file' (path in DB, copy in <db>-attachments/)", 'db')
    .option('--note <text>', 'optional note')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, async (ctx, db) => {
        const ref = resolveRef(opts);
        const result = addAttachment(db, {
          kind: ref.kind, refId: ref.id, filePath: opts.file, note: opts.note,
          store: opts.store, actor: ctx.actor, dryRun: ctx.dryRun,
        });
        output(ctx, result, (d) => {
          if (d.dryRun) {
            console.log(`plan: attach '${d.file_name}' (${fmtBytes(d.size)}, sha256 ${d.sha256.slice(0, 12)}…) to ${d.kind} ${d.ref_id} [${d.mode}]`);
            console.log('(dry run — nothing written)');
            return;
          }
          console.log(`attached ${d.file_name} (${fmtBytes(d.size)}) to ${d.kind} ${d.ref_id} as attachment ${d.id} [${d.mode}]`);
        });
    }));

  attach
    .command('list')
    .description('list attachments for an invoice or entry (metadata only)')
    .option('--invoice <id>', 'invoice id')
    .option('--entry <id>', 'journal entry id')
    .action((opts, command) => withDb(command, async (ctx, db) => {
        const ref = resolveRef(opts);
        const rows = listAttachments(db, { kind: ref.kind, refId: ref.id });
        output(ctx, { kind: ref.kind, ref_id: ref.id, attachments: rows }, (d) => {
          if (!d.attachments.length) {
            console.log(`no attachments for ${d.kind} ${d.ref_id}`);
            return;
          }
          table(d.attachments.map((a) => ({
            id: a.id, mode: a.mode, file: a.file_name, size: fmtBytes(a.size),
            sha: a.sha256.slice(0, 12), date: a.created_at.slice(0, 10), note: a.note ?? '',
          })), [
            { key: 'id', label: 'ID' },
            { key: 'mode', label: 'Mode' },
            { key: 'file', label: 'File' },
            { key: 'size', label: 'Size' },
            { key: 'sha', label: 'SHA256' },
            { key: 'date', label: 'Date' },
            { key: 'note', label: 'Note' },
          ]);
        });
    }));

  attach
    .command('show')
    .description('show attachment metadata, or extract the bytes with --out')
    .requiredOption('--id <id>', 'attachment id')
    .option('--out <path>', 'write the file to this path')
    .option('--force', 'overwrite an existing --out file')
    .action((opts, command) => withDb(command, async (ctx, db) => {
        const id = Number(opts.id);
        const row = getAttachment(db, id);
        if (opts.out) {
          if (existsSync(opts.out) && !opts.force) {
            throw dbError('FILE_EXISTS', `'${opts.out}' already exists — pass --force to overwrite`);
          }
          mkdirSync(path.dirname(path.resolve(opts.out)), { recursive: true });
          writeFileSync(opts.out, row.data);
          output(ctx, { id, file_name: row.file_name, out: opts.out, size: row.size, mode: row.mode }, (d) => {
            console.log(`wrote ${d.file_name} (${fmtBytes(d.size)}) to ${d.out}`);
          });
          return;
        }
        output(ctx, {
          id: row.id, kind: row.kind, ref_id: row.ref_id, file_name: row.file_name,
          mime: row.mime, size: row.size, sha256: row.sha256, mode: row.mode,
          path: row.path ?? null, note: row.note ?? null, created_by: row.created_by, created_at: row.created_at,
        }, (d) => {
          console.log(`attachment ${d.id}: ${d.file_name} (${fmtBytes(d.size)}, ${d.mime})`);
          console.log(`  kind: ${d.kind} ${d.ref_id}   mode: ${d.mode}   sha256: ${d.sha256}`);
          console.log(`  by ${d.created_by} on ${d.created_at.slice(0, 10)}`);
          if (d.note) console.log(`  note: ${d.note}`);
          if (d.mode === 'file') console.log(`  path: ${d.path}`);
          console.log('  (pass --out <path> to extract the file)');
        });
    }));

  attach
    .command('remove')
    .description('remove an attachment')
    .requiredOption('--id <id>', 'attachment id')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, async (ctx, db) => {
        const result = removeAttachment(db, { id: Number(opts.id), actor: ctx.actor, dryRun: ctx.dryRun });
        output(ctx, result, (d) => {
          if (d.dryRun) {
            console.log(`plan: remove attachment ${d.id} (${d.file_name}, ${d.kind} ${d.ref_id}${d.mode === 'file' ? ', file mode' : ''})`);
            console.log('(dry run — nothing written)');
            return;
          }
          console.log(`removed attachment ${d.id} (${d.file_name})`);
        });
    }));
}
