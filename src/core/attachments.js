/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// In-database document attachments (house rule 8 — the paper trail travels
// with `bukio backup`). mode 'db' (default) stores the file as a BLOB;
// mode 'file' copies it content-addressed into <db>-attachments/ and stores
// the path. Metadata-only list queries (never SELECT data) keep lists fast
// regardless of how many documents are stored.
import { existsSync, statSync, copyFileSync, unlinkSync, mkdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { record } from '../audit/index.js';

export const MAX_ATTACHMENT_BYTES = 25 * 1024 * 1024; // 25 MB per file

const MIME_BY_EXT = {
  '.pdf': 'application/pdf',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.png': 'image/png',
  '.gif': 'image/gif',
  '.svg': 'image/svg+xml',
  '.xml': 'application/xml',
  '.eml': 'message/rfc822',
  '.msg': 'application/vnd.ms-outlook',
  '.txt': 'text/plain',
  '.csv': 'text/csv',
  '.html': 'text/html',
  '.doc': 'application/msword',
  '.docx': 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  '.xls': 'application/vnd.ms-excel',
  '.xlsx': 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  '.zip': 'application/zip',
};

export function attachmentError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

/** <db>-attachments/ next to the database file (demo.db → demo-attachments/). */
export function attachmentsDir(dbPath) {
  return path.join(path.dirname(dbPath), `${path.basename(dbPath).replace(/\.db$/, '')}-attachments`);
}

function refExists(db, kind, refId) {
  const table = kind === 'invoice' ? 'invoices' : 'journal_entries';
  return Boolean(db.prepare(`SELECT id FROM ${table} WHERE id = ?`).get(refId));
}

/**
 * Store a document against an invoice or entry.
 * store 'db' (default): BLOB in the DB — travels with backups.
 * store 'file': copied to <db>-attachments/<sha256>, path stored.
 */
export function addAttachment(db, {
  kind, refId, filePath, note = null, store = 'db', actor = 'human', dryRun = false,
}) {
  if (!['invoice', 'entry'].includes(kind)) {
    throw attachmentError('INVALID_KIND', `attachment kind must be 'invoice' or 'entry', got '${kind}'`);
  }
  if (!Number.isInteger(refId) || refId <= 0) {
    throw attachmentError('REF_REQUIRED', 'pass exactly one of --invoice <id> or --entry <id>');
  }
  if (!['db', 'file'].includes(store)) {
    throw attachmentError('INVALID_STORE', `attachment store must be 'db' or 'file', got '${store}'`);
  }
  if (!refExists(db, kind, refId)) {
    throw attachmentError('NOT_FOUND', `${kind} ${refId} does not exist`);
  }
  if (!filePath || !existsSync(filePath)) {
    throw attachmentError('ATTACHMENT_FILE_NOT_FOUND', `file '${filePath}' does not exist`);
  }
  const size = statSync(filePath).size;
  if (size > MAX_ATTACHMENT_BYTES) {
    throw attachmentError('ATTACHMENT_TOO_LARGE', `file is ${size} bytes — the cap is ${MAX_ATTACHMENT_BYTES} (25 MB)`);
  }
  if (size < 1) {
    throw attachmentError('ATTACHMENT_EMPTY', 'file is empty — nothing to attach');
  }
  const bytes = readFileSync(filePath);
  const sha256 = createHash('sha256').update(bytes).digest('hex');
  const dup = db.prepare('SELECT id FROM attachments WHERE kind = ? AND ref_id = ? AND sha256 = ?').get(kind, refId, sha256);
  if (dup) {
    throw attachmentError('ATTACHMENT_DUPLICATE', `the same file is already attached (attachment ${dup.id})`);
  }
  const file_name = path.basename(filePath);
  const mime = MIME_BY_EXT[path.extname(file_name).toLowerCase()] ?? 'application/octet-stream';

  let dest = null;
  if (store === 'file') {
    dest = path.join(attachmentsDir(db.name), sha256);
  }

  if (dryRun) {
    return {
      action: 'attachments.add', kind, ref_id: refId, file_name, mime, size, sha256,
      mode: store, path: dest, dryRun: true,
    };
  }

  if (store === 'file') {
    mkdirSync(path.dirname(dest), { recursive: true });
    copyFileSync(filePath, dest);
  }

  try {
    const info = db.transaction(() => {
      const r = db.prepare(
        `INSERT INTO attachments (kind, ref_id, file_name, mime, size, sha256, mode, data, path, note, created_by)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(kind, refId, file_name, mime, size, sha256, store, store === 'db' ? bytes : null, dest, note, actor);
      record(db, {
        actor, action: 'attachments.add', command: 'attach add',
        args: { attachment_id: Number(r.lastInsertRowid), kind, ref_id: refId, file_name, size, mode: store, sha256 },
        outcome: 'ok',
      });
      return Number(r.lastInsertRowid);
    })();
    return { id: info, kind, ref_id: refId, file_name, mime, size, sha256, mode: store, path: dest, note, created_by: actor };
  } catch (err) {
    // file ops aren't transactional — clean up the copy if the insert failed
    if (dest) {
      try { unlinkSync(dest); } catch { /* already gone */ }
    }
    throw err;
  }
}

/** Metadata only — never SELECTs the data column, so lists stay fast. */
export function listAttachments(db, { kind, refId }) {
  return db.prepare(
    `SELECT id, kind, ref_id, file_name, mime, size, sha256, mode, path, note, created_by, created_at
     FROM attachments WHERE kind = ? AND ref_id = ? ORDER BY id`,
  ).all(kind, refId);
}

/** Full row incl. data (BLOB from the DB, or read from the file path). */
export function getAttachment(db, id) {
  const row = db.prepare('SELECT * FROM attachments WHERE id = ?').get(id);
  if (!row) throw attachmentError('ATTACHMENT_NOT_FOUND', `attachment ${id} does not exist`);
  if (row.mode === 'file') {
    if (!row.path || !existsSync(row.path)) {
      throw attachmentError('ATTACHMENT_FILE_MISSING', `attachment ${id} is file-mode but the file is missing at '${row.path}'`);
    }
    row.data = readFileSync(row.path);
  }
  return row;
}

export function removeAttachment(db, { id, actor = 'human', dryRun = false }) {
  const row = db.prepare('SELECT id, kind, ref_id, file_name, mode, path FROM attachments WHERE id = ?').get(id);
  if (!row) throw attachmentError('ATTACHMENT_NOT_FOUND', `attachment ${id} does not exist`);
  if (dryRun) {
    return { action: 'attachments.remove', id, kind: row.kind, ref_id: row.ref_id, file_name: row.file_name, mode: row.mode, dryRun: true };
  }
  db.transaction(() => {
    db.prepare('DELETE FROM attachments WHERE id = ?').run(id);
    record(db, {
      actor, action: 'attachments.remove', command: 'attach remove',
      args: { attachment_id: id, kind: row.kind, ref_id: row.ref_id, file_name: row.file_name, mode: row.mode },
      outcome: 'ok',
    });
  })();
  // the row is the truth — a stale file on disk is ignored, not an error
  if (row.mode === 'file' && row.path) {
    try { unlinkSync(row.path); } catch { /* already gone */ }
  }
  return { id, kind: row.kind, ref_id: row.ref_id, file_name: row.file_name, mode: row.mode };
}
