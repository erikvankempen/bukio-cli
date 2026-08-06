// DB layer — open, pragmas, migrations.
import Database from 'better-sqlite3';
import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const MIGRATIONS_DIR = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  '..', '..', 'migrations',
);

export function openDb(dbPath) {
  const db = new Database(dbPath);
  db.pragma('journal_mode = WAL');
  db.pragma('foreign_keys = ON');
  db.pragma('busy_timeout = 5000');
  migrate(db);
  return db;
}

export function migrate(db) {
  const current = db.pragma('user_version', { simple: true });
  const files = readdirSync(MIGRATIONS_DIR)
    .filter((f) => /^\d+_.*\.sql$/.test(f))
    .sort((a, b) => parseInt(a, 10) - parseInt(b, 10));
  for (const file of files) {
    const version = parseInt(file.split('_')[0], 10);
    if (version <= current) continue;
    const sql = readFileSync(path.join(MIGRATIONS_DIR, file), 'utf8');
    // Rebuild migrations (table rebuilds that must toggle PRAGMA foreign_keys)
    // declare `PRAGMA foreign_keys = OFF` and run OUTSIDE a transaction — the
    // pragma is a no-op inside one.
    if (/PRAGMA foreign_keys/i.test(sql)) {
      const wasOn = db.pragma('foreign_keys', { simple: true }) === 1;
      db.pragma('foreign_keys = OFF');
      try {
        db.exec(sql);
        db.pragma(`user_version = ${version}`);
      } catch (err) {
        throw new Error(`migration ${file} failed: ${err.message}`);
      } finally {
        db.pragma(`foreign_keys = ${wasOn ? 'ON' : 'OFF'}`);
      }
      continue;
    }
    db.exec('BEGIN');
    try {
      db.exec(sql);
      db.pragma(`user_version = ${version}`);
      db.exec('COMMIT');
    } catch (err) {
      db.exec('ROLLBACK');
      throw new Error(`migration ${file} failed: ${err.message}`);
    }
  }
}
