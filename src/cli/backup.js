/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio backup / restore — SQLite backup API + validated restore.
// --encrypt wraps the backup in AES-256-GCM (scrypt key from a passphrase:
// --passphrase flag or BUKIO_BACKUP_PASSPHRASE env). Encrypted file layout:
//   "BUKIOENC1" | salt(16) | iv(12) | tag(16) | ciphertext
// --keep N prunes oldest backups in the DEFAULT backup folder (~/.bukio/backups).
import {
  copyFileSync, existsSync, mkdirSync, statSync, readdirSync, unlinkSync,
  openSync, readSync, closeSync, readFileSync, writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createCipheriv, createDecipheriv, randomBytes, scryptSync } from 'node:crypto';
import Database from 'better-sqlite3';
import { ensureDb, makeCtx, output, fail, dbError } from './util.js';
import { record } from '../audit/index.js';

export const BACKUP_MAGIC = 'BUKIOENC1';
const KEY_LEN = 32; // AES-256
const SALT_LEN = 16;
const IV_LEN = 12;
const TAG_LEN = 16;

function defaultBackupPath(encrypted = false) {
  const ts = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
  return path.join(os.homedir(), '.bukio', 'backups', `bukio-${ts}.db${encrypted ? '.enc' : ''}`);
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

/** Passphrase from --passphrase or BUKIO_BACKUP_PASSPHRASE; throws if absent. */
export function backupPassphrase(opts) {
  const pass = opts.passphrase ?? process.env.BUKIO_BACKUP_PASSPHRASE ?? null;
  if (!pass) {
    throw dbError('BACKUP_PASSPHRASE_REQUIRED', 'pass --passphrase or set BUKIO_BACKUP_PASSPHRASE (needed for --encrypt / encrypted restore)');
  }
  return pass;
}

/** True when the file starts with the BUKIOENC1 magic (encrypted backup). */
export function isEncryptedBackup(filePath) {
  const fd = openSync(filePath, 'r');
  try {
    const buf = Buffer.alloc(BACKUP_MAGIC.length);
    const n = readSync(fd, buf, 0, buf.length, 0);
    return n === buf.length && buf.toString('latin1') === BACKUP_MAGIC;
  } finally {
    closeSync(fd);
  }
}

/** Encrypt a plain SQLite backup file into `dest`; returns the dest size. */
export function encryptBackupFile(src, dest, passphrase) {
  const salt = randomBytes(SALT_LEN);
  const iv = randomBytes(IV_LEN);
  const key = scryptSync(passphrase, salt, KEY_LEN, { N: 2 ** 15, r: 8, p: 1, maxmem: 64 * 1024 * 1024 });
  const cipher = createCipheriv('aes-256-gcm', key, iv);
  const enc = Buffer.concat([cipher.update(readFileSync(src)), cipher.final()]);
  writeFileSync(dest, Buffer.concat([Buffer.from(BACKUP_MAGIC, 'latin1'), salt, iv, cipher.getAuthTag(), enc]));
  return statSync(dest).size;
}

/** Decrypt an encrypted backup into a Buffer; wrong passphrase → BACKUP_PASSPHRASE_WRONG. */
export function decryptBackupFile(src, passphrase) {
  const raw = readFileSync(src);
  if (raw.toString('latin1', 0, BACKUP_MAGIC.length) !== BACKUP_MAGIC) {
    throw dbError('INVALID_BACKUP', `'${src}' is not an encrypted bukio backup`);
  }
  let off = BACKUP_MAGIC.length;
  const salt = raw.subarray(off, off + SALT_LEN); off += SALT_LEN;
  const iv = raw.subarray(off, off + IV_LEN); off += IV_LEN;
  const tag = raw.subarray(off, off + TAG_LEN); off += TAG_LEN;
  const key = scryptSync(passphrase, salt, KEY_LEN, { N: 2 ** 15, r: 8, p: 1, maxmem: 64 * 1024 * 1024 });
  const decipher = createDecipheriv('aes-256-gcm', key, iv);
  decipher.setAuthTag(tag);
  try {
    return Buffer.concat([decipher.update(raw.subarray(off)), decipher.final()]);
  } catch (err) {
    throw dbError('BACKUP_PASSPHRASE_WRONG', 'wrong passphrase or corrupted backup — decryption failed');
  }
}

/**
 * Rotation: prune oldest backups in the DEFAULT backup folder, keeping the
 * newest `keep`. Only bukio-*.db / bukio-*.db.enc files count. Returns
 * { dir, pruned } (pruned = file names); dryRun deletes nothing.
 */
export function pruneBackups(keep, { dryRun = false } = {}) {
  const dir = path.join(os.homedir(), '.bukio', 'backups');
  if (!existsSync(dir)) return { dir, pruned: [] };
  const files = readdirSync(dir)
    .filter((f) => /^bukio-.*\.db(\.enc)?$/.test(f))
    .map((f) => ({ name: f, mtime: statSync(path.join(dir, f)).mtimeMs }))
    // newest first; filename tiebreaker keeps the sort deterministic when
    // several backups share the same mtime (ms granularity)
    .sort((a, b) => (b.mtime - a.mtime) || b.name.localeCompare(a.name));
  const pruned = files.slice(keep).map((f) => f.name);
  if (!dryRun) {
    for (const f of pruned) {
      try { unlinkSync(path.join(dir, f)); } catch { /* raced */ }
    }
  }
  return { dir, pruned };
}

export function make(program) {
  program
    .command('backup')
    .description('create a consistent SQLite backup of the database (optionally encrypted)')
    .option('--out <path>', 'backup file (default ~/.bukio/backups/bukio-<ts>.db)')
    .option('--encrypt', 'AES-256-GCM encrypt the backup (needs --passphrase or BUKIO_BACKUP_PASSPHRASE)')
    .option('--passphrase <pass>', 'encryption passphrase (or BUKIO_BACKUP_PASSPHRASE env)')
    .option('--keep <n>', 'prune oldest backups in the default backup folder, keeping the newest N (no --out)')
    .option('--dry-run', 'show the plan without writing')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const encrypted = Boolean(opts.encrypt);
        const pass = encrypted ? backupPassphrase(opts) : null;
        const keep = opts.keep !== undefined ? Number(opts.keep) : null;
        if (keep !== null && (!Number.isInteger(keep) || keep < 1)) {
          throw dbError('INVALID_KEEP', `--keep must be a positive integer, got '${opts.keep}'`);
        }
        if (keep !== null && opts.out) {
          throw dbError('INVALID_KEEP', '--keep rotation applies to the default backup folder only — drop --out or drop --keep');
        }
        const dest = opts.out || defaultBackupPath(encrypted);
        const finalDest = encrypted && !dest.endsWith('.enc') ? `${dest}.enc` : dest;
        // dry-run: report what the rotation WOULD prune (nothing written)
        const dryRunRotation = ctx.dryRun && keep !== null ? pruneBackups(keep, { dryRun: true }) : null;
        if (ctx.dryRun) {
          output(ctx, {
            action: 'backup', from: ctx.dbPath, to: finalDest, encrypted,
            pruned: dryRunRotation ? dryRunRotation.pruned : [],
            dryRun: true,
          }, (d) => {
            console.log(`plan: backup ${d.from} -> ${d.to}${d.encrypted ? ' (encrypted)' : ''}`);
            if (d.pruned.length) console.log(`plan: prune ${d.pruned.length} old backup(s) in ${dryRunRotation.dir}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        const db = ensureDb(ctx);
        mkdirSync(path.dirname(path.resolve(finalDest)), { recursive: true });
        let bytes;
        if (encrypted) {
          const plain = `${finalDest}.plain`;
          try {
            await db.backup(plain);
            bytes = encryptBackupFile(plain, finalDest, pass);
          } finally {
            try { unlinkSync(plain); } catch { /* already gone */ }
          }
        } else {
          await db.backup(finalDest);
          bytes = statSync(finalDest).size;
        }
        record(db, {
          actor: ctx.actor, action: 'backup', command: 'backup',
          args: { to: finalDest, bytes, encrypted }, outcome: 'ok',
        });
        db.close();
        // prune AFTER the new backup exists — pruning first always left
        // N+1 files (N kept + the one just created)
        const rotation = keep !== null ? pruneBackups(keep) : null;
        output(ctx, { path: finalDest, bytes, source: ctx.dbPath, encrypted, pruned: rotation ? rotation.pruned : [] }, (d) => {
          console.log(`backup written: ${d.path} (${d.bytes} bytes${d.encrypted ? ', encrypted' : ''})`);
          if (d.pruned.length) console.log(`pruned ${d.pruned.length} old backup(s)`);
        });
      } catch (err) {
        fail(ctx, err);
      }
    });

  program
    .command('restore')
    .description('restore a database from a backup file (encrypted backups auto-detected)')
    .requiredOption('--from <file>', 'backup file to restore')
    .option('--to <path>', 'target database (default: the active --db)')
    .option('--passphrase <pass>', 'encryption passphrase (or BUKIO_BACKUP_PASSPHRASE env)')
    .option('--force', 'overwrite an existing initialised database')
    .option('--dry-run', 'validate and show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      let tempPath = null;
      try {
        if (!existsSync(opts.from)) {
          throw dbError('FILE_NOT_FOUND', `backup file '${opts.from}' does not exist`);
        }
        const encrypted = isEncryptedBackup(opts.from);
        const pass = encrypted ? backupPassphrase(opts) : null;
        const src = opts.from;
        if (encrypted) {
          tempPath = path.join(os.tmpdir(), `bukio-restore-${process.pid}-${Date.now()}.db`);
          writeFileSync(tempPath, decryptBackupFile(src, pass));
        }
        validateBackupFile(tempPath || src);
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
          output(ctx, {
            action: 'restore', from: opts.from, to: target, encrypted, dryRun: true,
          }, (d) => {
            console.log(`plan: restore ${d.from} -> ${d.to}${d.encrypted ? ' (encrypted)' : ''}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        copyFileSync(tempPath || src, target);
        // record the restore INTO the restored DB (the trail names who acted)
        const restoredDb = new Database(target);
        try {
          record(restoredDb, {
            actor: ctx.actor, action: 'restore', command: 'restore',
            args: { from: opts.from, to: target, encrypted }, outcome: 'ok',
          });
        } finally {
          restoredDb.close();
        }
        output(ctx, { to: target, from: opts.from, encrypted, restored: true }, (d) => {
          console.log(`restored ${d.from} -> ${d.to}${d.encrypted ? ' (decrypted)' : ''}`);
        });
      } catch (err) {
        fail(ctx, err);
      } finally {
        if (tempPath) {
          try { unlinkSync(tempPath); } catch { /* already gone */ }
        }
      }
    });
}
