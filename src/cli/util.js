/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// CLI helpers — context, JSON/human output, db access, actor signing.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { openDb } from '../core/db.js';
import { buildDigest } from '../core/canonical.js';
import {
  sign, verify, keyidOf, isEncrypted, publicKeyFromPrivate, decryptPrivateKey,
} from '../core/sign.js';
import { getActorKey, getAnyActorKey, getEnforce } from '../core/actor-registry.js';
import { checkAuthz } from '../core/authz.js';

export function dbError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

// --- actor key/session paths (shared by the actor CLI and the sign gate) ---

/** Config dir for keys/sessions/nonces: BUKIO_CONFIG_DIR or ~/.bukio. */
export function configDir() {
  return process.env.BUKIO_CONFIG_DIR || path.join(os.homedir(), '.bukio');
}

export function keyFilePath(actor) {
  return path.join(configDir(), 'keys', `${actor.replace(':', '-')}.key`);
}

export function sessionFilePath(actor) {
  return path.join(configDir(), 'sessions', `${actor.replace(':', '-')}.key`);
}

/**
 * Read the short-lived session key for an actor. Returns
 * { keyPem, expiresAt } or null when missing, malformed or expired
 * (expired counts as locked).
 */
export function readSessionKey(actor) {
  try {
    const raw = JSON.parse(readFileSync(sessionFilePath(actor), 'utf8'));
    if (!raw.keyPem || !raw.expiresAt) return null;
    if (new Date(raw.expiresAt).getTime() <= Date.now()) return null;
    return { keyPem: raw.keyPem, expiresAt: raw.expiresAt };
  } catch {
    return null;
  }
}

/** Build the shared command context from global+local options. */
export function makeCtx(command) {
  const o = command.optsWithGlobals();
  return {
    json: Boolean(o.json),
    dbPath: o.db,
    actor: o.actor ?? process.env.BUKIO_ACTOR ?? null,
    dryRun: Boolean(o.dryRun),
    server: o.server ?? process.env.BUKIO_SERVER ?? null,
    locale: o.locale ?? process.env.BUKIO_LOCALE ?? null,
  };
}

/** Emit results: JSON (stable schema) or human-rendered text. */
export function output(ctx, data, render) {
  if (ctx.json) {
    console.log(JSON.stringify({ ok: true, data }, null, 2));
  } else {
    render(data);
  }
}

/** Emit errors: JSON { ok:false, error:{code,message,details?} } or stderr lines. */
export function fail(ctx, err) {
  const code = err.code || 'ERROR';
  if (ctx.json) {
    const error = { code, message: err.message };
    if (err.details) error.details = err.details;
    console.log(JSON.stringify({ ok: false, error }, null, 2));
  } else {
    console.error(`error [${code}]: ${err.message}`);
    if (err.details) {
      for (const d of err.details) {
        console.error(`  line ${d.line || '-'}: ${d.error}`);
      }
    }
  }
  process.exitCode = 1;
}

/**
 * Open the database. create=true → mkdir + create file (for init).
 * mustExist=true (default) → throw NO_DATABASE when the file is absent.
 * dryRun handled by callers: pass mustExist=false and open only if present.
 */
export function ensureDb(ctx, { create = false, mustExist = true } = {}) {
  const exists = existsSync(ctx.dbPath);
  if (!exists) {
    if (create) {
      mkdirSync(path.dirname(ctx.dbPath), { recursive: true });
    } else if (mustExist) {
      throw dbError('NO_DATABASE', `no database at ${ctx.dbPath} — run 'bukio init' first`);
    } else {
      // tolerate a missing DB — do NOT open it: openDb() would CREATE the
      // file and run every migration (even in --dry-run), and a missing
      // parent dir would throw SQLITE_CANTOPEN instead of "no database"
      return null;
    }
  }
  return openDb(ctx.dbPath);
}

/** Simple aligned text table for human output. cols: [{ key, label }] */
export function table(rows, cols) {
  const widths = cols.map((c) => Math.max(
    c.label.length,
    ...rows.map((r) => String(r[c.key] ?? '').length),
  ));
  const line = (cells) => cells.map((c, i) => String(c).padEnd(widths[i])).join('  ');
  console.log(line(cols.map((c) => c.label)));
  console.log(widths.map((w) => '-'.repeat(w)).join('  '));
  for (const r of rows) console.log(line(cols.map((c) => r[c.key] ?? '')));
}

/** Global transport flags stripped from a transmitted argv (the server's DB
 * and URL are authoritative; the signing key stays local). Shared by the
 * remote client (before signing) and the remote server (defense in depth —
 * a hostile envelope's argv must not redirect the child to another DB). */
export const TRANSPORT_FLAGS = new Set(['--server', '--db', '--sign-key']);

/**
 * Remove transport flags (+ their values) from a raw argv. Supports both
 * `--flag value` and `--flag=value` forms. Everything after a bare `--`
 * is positional and kept verbatim.
 */
export function sanitizeArgv(argv) {
  const out = [];
  let positional = false;
  for (let i = 0; i < argv.length; i++) {
    const tok = argv[i];
    if (!positional && tok === '--') {
      positional = true;
      out.push(tok);
      continue;
    }
    if (!positional) {
      const eq = tok.startsWith('--') && tok.includes('=') ? tok.indexOf('=') : -1;
      const flag = eq > -1 ? tok.slice(0, eq) : tok;
      if (TRANSPORT_FLAGS.has(flag)) {
        if (eq > -1) continue; // --server=URL form: drop the whole token
        // --flag value form: drop the next token too IF it is not another flag
        if (i + 1 < argv.length && !argv[i + 1].startsWith('-')) i++;
        continue;
      }
    }
    out.push(tok);
  }
  return out;
}

// --- sign-and-verify gate (Tier 0) -----------------------------------------
//
// Every command is digitally signed by its declared actor and verified
// against the per-company registry (actor_keys) before dispatch. Three
// outcomes per command:
//   - 'verified': a key was available, the actor is enrolled, and the
//     signature verifies (timestamp window ±5 min, nonce not reused).
//   - 'unsigned': no key material / not enrolled / unverifiable — the
//     command RUNS (record mode, the default) but is logged honestly as
//     "claimed, not yet provable".
//   - refusal: under enforcement (settings.signing_enforce = 'on') any of
//     those conditions aborts the command with an error code before
//     anything is mutated. NONCE_REUSED refuses in both modes (replay
//     evidence is never tolerated).

const SIGNATURE_WINDOW_MS = 5 * 60_000; // ±5 minutes
const NONCE_RETENTION_MS = 24 * 3600_000; // nonces remembered 24 h

export function signGateError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

function noncesPath() {
  return path.join(configDir(), 'nonces.json');
}

function readNonces() {
  try {
    return JSON.parse(readFileSync(noncesPath(), 'utf8'));
  } catch {
    return {};
  }
}

export function isNonceUsed(keyid, nonce) {
  const byKey = readNonces()[keyid];
  return byKey ? Object.prototype.hasOwnProperty.call(byKey, nonce) : false;
}

export function rememberNonce(keyid, nonce) {
  const now = new Date();
  const cutoff = new Date(Date.now() - NONCE_RETENTION_MS).toISOString();
  const nonces = readNonces();
  for (const [k, byKey] of Object.entries(nonces)) {
    for (const [n, ts] of Object.entries(byKey)) {
      if (ts < cutoff) delete byKey[n];
    }
    if (Object.keys(byKey).length === 0) delete nonces[k];
  }
  nonces[keyid] = { ...(nonces[keyid] ?? {}), [nonce]: now.toISOString() };
  mkdirSync(path.dirname(noncesPath()), { recursive: true, mode: 0o700 });
  writeFileSync(noncesPath(), JSON.stringify(nonces), { mode: 0o600 });
}

/**
 * Verify a signature bundle against the company registry.
 *
 * @param {object|null} db - open company DB, or null (no DB -> record semantics).
 * @param {object} b
 * @param {string} b.actor - declared actor.
 * @param {string} b.digest - sha256 hex digest that was signed.
 * @param {string} b.sig - base64 signature.
 * @param {string} b.keyid - keyid of the key that produced the signature.
 * @param {string} b.ts - ISO timestamp of the signature.
 * @param {string} b.nonce - one-time token.
 * @param {boolean} b.enforce - true refuses on any anomaly (except nonce
 *   reuse, which always refuses).
 * @returns {{ok: boolean, status: 'verified'|'unsigned', code?: string}}
 */
export function verifySignatureBundle(db, { actor, digest, sig, keyid, ts, nonce, enforce }) {
  let row = db ? getActorKey(db, actor) : null; // the active key
  if (!row && db) {
    // no active key: distinguish 'never enrolled' from 'enrolled, since revoked'
    const any = getAnyActorKey(db, actor);
    if (any && any.revoked_at !== null) {
      if (enforce) return { ok: false, status: 'unsigned', code: 'ACTOR_KEY_REVOKED' };
      return { ok: true, status: 'unsigned' };
    }
  }
  if (!row) {
    if (enforce) return { ok: false, status: 'unsigned', code: 'ACTOR_KEY_UNKNOWN' };
    return { ok: true, status: 'unsigned' };
  }
  const tsMs = Date.parse(ts);
  if (Number.isNaN(tsMs) || Math.abs(Date.now() - tsMs) > SIGNATURE_WINDOW_MS) {
    if (enforce) return { ok: false, status: 'unsigned', code: 'SIGNATURE_STALE' };
    return { ok: true, status: 'unsigned' };
  }
  if (isNonceUsed(row.keyid, nonce)) return { ok: false, status: 'unsigned', code: 'NONCE_REUSED' };
  if (row.keyid !== keyid || !verify(digest, sig, row.public_key)) {
    if (enforce) return { ok: false, status: 'unsigned', code: 'SIGNATURE_INVALID' };
    return { ok: true, status: 'unsigned' };
  }
  rememberNonce(row.keyid, nonce);
  return { ok: true, status: 'verified' };
}

/**
 * Resolve the signing key material for an actor. Priority: --sign-key path
 * (explicit), then the session key (human unlock), then the key file —
 * with BUKIO_SIGNING_PASSPHRASE decrypting passphrase-encrypted keys
 * in-memory (never written to disk).
 *
 * @param {string} actor
 * @param {string|null} signKeyPath - explicit key file from --sign-key.
 * @param {boolean} enforce - true -> missing material throws instead of
 *   returning null (record mode runs unsigned).
 * @returns {{keyPem: string, keyid: string}|null}
 * @throws SIGNATURE_REQUIRED / KEY_NOT_FOUND / PASSPHRASE_REQUIRED /
 *   PASSPHRASE_INVALID when enforce is on.
 */
export function resolveSigningKey(actor, signKeyPath, enforce) {
  const file = signKeyPath ?? keyFilePath(actor);
  if (!signKeyPath) {
    const session = readSessionKey(actor);
    if (session) {
      return { keyPem: session.keyPem, keyid: keyidOf(publicKeyFromPrivate(session.keyPem)) };
    }
  }
  if (!existsSync(file)) {
    if (enforce) {
      throw signGateError(
        signKeyPath ? 'KEY_NOT_FOUND' : 'SIGNATURE_REQUIRED',
        signKeyPath
          ? `signing key ${file} not found`
          : `no key material for ${actor} — run 'bukio actor keygen' + 'actor register' (and 'actor unlock' for human keys)`,
      );
    }
    return null;
  }
  const pem = readFileSync(file, 'utf8');
  if (isEncrypted(pem)) {
    const passphrase = process.env.BUKIO_SIGNING_PASSPHRASE;
    if (!passphrase) {
      if (enforce) {
        throw signGateError('PASSPHRASE_REQUIRED',
          `the key for ${actor} is passphrase-encrypted — run 'bukio actor unlock' or set BUKIO_SIGNING_PASSPHRASE`);
      }
      return null;
    }
    try {
      return { keyPem: decryptPrivateKey(pem, { passphrase }), keyid: keyidOf(publicKeyFromPrivate(pem, { passphrase })) };
    } catch {
      if (enforce) throw signGateError('PASSPHRASE_INVALID', 'wrong passphrase');
      return null;
    }
  }
  return { keyPem: pem, keyid: keyidOf(publicKeyFromPrivate(pem)) };
}

/** Bootstrap commands that must work without a valid signature. */
export function isSigningExempt(commandPath, opts) {
  // key CREATION and session handling are pure bootstrap (files only, no
  // registry state): keygen writes a key file, unlock/lock manage the
  // session. `actor register` is NOT exempt here — it mutates the company
  // registry, so its exemption is decided in signCommand() and limited to
  // rotation re-enrolment (actor currently revoked). `actor enforce --off`
  // is NOT exempt either — only an enrolled actor may disable enforcement.
  if (['actor keygen', 'actor unlock', 'actor lock'].includes(commandPath)) return true;
  // the MCP server command starts a bridge — it mutates nothing itself, and
  // every mutating tool call is signed individually (mcp: gate in mcp.js),
  // so under enforce the server must still be able to START
  if (commandPath === 'mcp') return true;
  // the remote server command starts a bridge too — every /rpc command is
  // signed individually, so the daemon itself needs no signature; `server
  // token` mints single-use enrolment tokens (bootstrap, like keygen — it
  // must work before any key exists in the company registry)
  if (commandPath === 'server start' || commandPath === 'server token') return true;
  return false;
}

/** Full command path without the program name: 'entry add', 'actor keygen'.
 * (commander 13 removed commandPath(); walk the parent chain instead.) */
export function commandPathOf(command) {
  const parts = [];
  for (let c = command; c; c = c.parent) parts.unshift(c.name());
  return parts[0] === 'bukio' ? parts.slice(1).join(' ') : parts.join(' ');
}

/**
 * Signed args = commander options minus identity/output flags, plus positionals.
 * The identity flag `--actor` is stripped ONLY when its value is the acting
 * actor itself (or unset): commands like `actor roles grant --actor <who>`
 * use `--actor` as a MEANINGFUL option naming the grant target, and the
 * signed payload must cover it (otherwise the audit row would not document
 * WHO was granted — a tamper-evidence hole).
 *
 * @param {object} opts - commander opts (optsWithGlobals).
 * @param {object} command - the commander action command.
 * @param {string} ctxActor - the acting actor.
 * @param {string[]} [extraExclude=[]] - additional option keys to strip
 *   (remote mode strips transport-level `db`/`server`: the server's DB is
 *   authoritative, so the client's --db/--server are not part of the
 *   operation's semantics — same rationale as actor/signKey/json).
 * @returns {object} the signed args object.
 */
export function buildSignedArgs(opts, command, ctxActor, extraExclude = []) {
  const args = {};
  for (const [k, v] of Object.entries(opts)) {
    if (k === 'signKey' || k === 'json') continue;
    if (k === 'actor' && (v === undefined || v === ctxActor)) continue;
    if (extraExclude.includes(k)) continue;
    args[k] = v;
  }
  if (command.args && command.args.length) args.positionals = command.args;
  return args;
}

/** Open the company DB for the gate if it exists (never create; tolerate a
 * missing/corrupt file — the command itself will report NO_DATABASE later). */
function openIfExists(dbPath) {
  if (!existsSync(dbPath)) return null;
  try {
    return openDb(dbPath);
  } catch {
    return null;
  }
}

/**
 * Build and verify the signed-command bundle for a commander action.
 * Returns the audit-row signature fields, or throws a sign-gate error
 * (enforcement refusal) before any mutation happens.
 *
 * @param {object} ctx - { json, dbPath, actor, dryRun } from makeCtx.
 * @param {object} command - the commander action command.
 * @returns {{digestHash, sigKeyid, sigNonce, sigTs, sig, sigStatus}}
 */
export function signCommand(ctx, command) {
  const opts = command.optsWithGlobals();
  const commandPath = commandPathOf(command);
  if (isSigningExempt(commandPath, opts)) return { sigStatus: 'unsigned' };
  const db = openIfExists(ctx.dbPath);
  try {
    // rotation re-enrolment: `actor register` for an actor whose key was
    // REVOKED (no active key) is the ONLY way back in — exempt it. First-
    // time enrolment under enforce is REFUSED (ACTOR_KEY_UNKNOWN): a brand-
    // new actor must be registered while enforce is off (the audited
    // recovery hatch), so enrolment stays an operator-gated act.
    if (commandPath === 'actor register' && db) {
      const any = getAnyActorKey(db, ctx.actor);
      if (!any || any.revoked_at === null) {
        return signPayload(ctx, {
          cmd: commandPath,
          args: buildSignedArgs(opts, command, ctx.actor),
          db,
          signKey: opts.signKey ?? null,
        });
      }
      return { sigStatus: 'unsigned' };
    }
    return signPayload(ctx, {
      cmd: commandPath,
      args: buildSignedArgs(opts, command, ctx.actor),
      db,
      signKey: opts.signKey ?? null,
    });
  } finally {
    if (db) db.close();
  }
}

/**
 * Build and verify a signed payload (CLI command or MCP tool call) and
 * return the audit-row signature fields. Record mode with no key material
 * returns { sigStatus: 'unsigned' }; enforcement refusals throw a
 * sign-gate error BEFORE any mutation can happen. After the signature
 * verifies, the Tier 0.5 authz gate runs (checkAuthz): under authz mode
 * the actor needs a role granting the command's capability, or the
 * command is refused with AUTHZ_DENIED — dry-run included (a plan is
 * refused the same way the real run is).
 *
 * @param {object} ctx - { actor } — the actor the payload is signed by.
 * @param {object} opts
 * @param {string} opts.cmd - canonical command string ('entry add',
 *   'mcp:entry_add') — stored on the audit row as `command`.
 * @param {object} opts.args - the exact signed args (stored as args_json).
 * @param {object|null} opts.db - open company DB for the registry/enforce
 *   check (null = no DB: record mode).
 * @param {string|null} [opts.signKey] - explicit key path (CLI --sign-key).
 * @returns {{digestHash, sigKeyid, sigNonce, sigTs, sig, sigStatus,
 *   signedArgs, signedCommand}}
 */
export function signPayload(ctx, { cmd, args, db, signKey = null }) {
  const ts = new Date().toISOString();
  const nonce = crypto.randomUUID();
  const digest = buildDigest({ actor: ctx.actor, cmd, args, ts, nonce });
  const enforce = db ? getEnforce(db) === 'on' : false;
  const key = resolveSigningKey(ctx.actor, signKey, enforce);
  if (!key) return { sigStatus: 'unsigned' };
  const sig = sign(digest, key.keyPem);
  const result = verifySignatureBundle(db, {
    actor: ctx.actor, digest, sig, keyid: key.keyid, ts, nonce, enforce,
  });
  if (!result.ok) throw signGateError(result.code, messageFor(result.code, ctx.actor));
  // Tier 0.5 authz gate: after the signature verifies, refuse before any
  // mutation when the actor lacks the command's capability (dry-run
  // included — D6). Unmapped commands deny (fail closed).
  checkAuthz(db, ctx.actor, cmd, args);
  return {
    digestHash: digest, sigKeyid: key.keyid, sigNonce: nonce, sigTs: ts, sig,
    sigStatus: result.status, signedArgs: args, signedCommand: cmd,
  };
}

function messageFor(code, actor) {
  const messages = {
    ACTOR_KEY_UNKNOWN: `actor ${actor} has no enrolled key in this company's DB — run 'bukio actor register' (a FIRST enrolment requires enforcement to be off: 'actor enforce --off', register, 'actor enforce --on')`,
    ACTOR_KEY_REVOKED: `the key for ${actor} is revoked in this company's DB — rotate with 'bukio actor keygen --force' + 'actor register'`,
    SIGNATURE_STALE: 'signature timestamp is outside the ±5 minute window',
    NONCE_REUSED: 'signature nonce was already used — a replayed command is refused',
    SIGNATURE_INVALID: `signature does not verify against the enrolled key for ${actor}`,
  };
  return messages[code] ?? 'signature verification failed';
}
