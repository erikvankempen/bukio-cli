/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio backup / restore — SQLite backup API + validated restore.
import { copyFileSync, existsSync, mkdirSync, statSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import Database from 'better-sqlite3';
import { ensureDb, makeCtx, output, fail, dbError } from './util.js';

function defaultBackupPath() {
  const ts = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
  return path.join(os.homedir(), '.bukio', 'backups', `bukio-${ts}.db`);
}

function validateBackupFile(filePath) {
  let db;
  try {
    db = new Database(filePath, { readonly: true });
    const version = db.pragma('user_version', { simple: true });
    const company = db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='company'").get();
    if (!company || version < 1) {
      throw dbError('INVALID_BACKUP', `'${filePath}' is not a bukio database (user_version=${version})`);
    }
  } catch (err) {
    if (err && err.code === 'INVALID_BACKUP') throw err;
    throw dbError('INVALID_BACKUP', `'${filePath}' is not a valid SQLite database`);
  } finally {
    if (db) db.close();
  }
}

export function make(program) {
  program
    .command('backup')
    .description('create a consistent SQLite backup of the database')
    .option('--out <path>', 'backup file (default ~/.bukio/backups/bukio-<ts>.db)')
    .option('--dry-run', 'show the plan without writing')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        const dest = opts.out || defaultBackupPath();
        if (ctx.dryRun) {
          db.close();
          output(ctx, { action: 'backup', from: ctx.dbPath, to: dest, dryRun: true }, (d) => {
            console.log(`plan: backup ${d.from} -> ${d.to}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        mkdirSync(path.dirname(path.resolve(dest)), { recursive: true });
        await db.backup(dest);
        db.close();
        const bytes = statSync(dest).size;
        output(ctx, { path: dest, bytes, source: ctx.dbPath }, (d) => {
          console.log(`backup written: ${d.path} (${d.bytes} bytes)`);
        });
      } catch (err) {
        fail(ctx, err);
      }
    });

  program
    .command('restore')
    .description('restore a database from a backup file')
    .requiredOption('--from <file>', 'backup file to restore')
    .option('--to <path>', 'target database (default: the active --db)')
    .option('--force', 'overwrite an existing initialised database')
    .option('--dry-run', 'validate and show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        if (!existsSync(opts.from)) {
          throw dbError('FILE_NOT_FOUND', `backup file '${opts.from}' does not exist`);
        }
        validateBackupFile(opts.from);
        const target = opts.to || ctx.dbPath;
        if (path.resolve(target) === path.resolve(opts.from)) {
          throw dbError('SAME_FILE', 'source and target are the same file');
        }
        if (existsSync(target)) {
          let hasCompany = false;
          try {
            const existing = new Database(target, { readonly: true });
            hasCompany = Boolean(existing.prepare('SELECT id FROM company').get());
            existing.close();
          } catch {
            // not a valid DB — overwrite freely
          }
          if (hasCompany && !opts.force) {
            throw dbError('RESTORE_EXISTS', `target ${target} already has a company — pass --force to overwrite`);
          }
        }
        if (ctx.dryRun) {
          output(ctx, { action: 'restore', from: opts.from, to: target, dryRun: true }, (d) => {
            console.log(`plan: restore ${d.from} -> ${d.to}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        copyFileSync(opts.from, target);
        output(ctx, { to: target, from: opts.from, restored: true }, (d) => {
          console.log(`restored ${d.from} -> ${d.to}`);
        });
      } catch (err) {
        fail(ctx, err);
      }
    });
}
