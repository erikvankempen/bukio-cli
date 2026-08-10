/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio actor — Tier 0 key-bound actor identity: keygen/register/list/
// revoke/enforce/unlock/lock/verify. Keys live under the config dir
// (BUKIO_CONFIG_DIR or ~/.bukio): keys/<role>-<name>.key (0600, dir 0700),
// sessions/<role>-<name>.key for short-lived human unlocks. The registry
// itself is per-company (actor_keys table in the company DB).
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import readline from 'node:readline';
import { ensureDb, makeCtx, output, fail, table } from './util.js';
import {
  generateKeyPair, keyidOf, isEncrypted, publicKeyFromPrivate, decryptPrivateKey,
} from '../core/sign.js';
import { isValidActor } from '../core/actor.js';
import {
  enrolActor, revokeActor, getActorKey, setEnforce, getEnforce, canAct,
} from '../core/actor-registry.js';
import { record } from '../audit/index.js';

const DEFAULT_TTL_HOURS = 12;
const MAX_TTL_HOURS = 168; // 7 days

export function actorCliError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

/** Config dir for keys/sessions: BUKIO_CONFIG_DIR or ~/.bukio. */
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
 * (expired counts as locked — the sign gate refuses on null).
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

function writePrivateKeyFile(file, pem) {
  mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  writeFileSync(file, pem, { mode: 0o600 });
}

/** Read a passphrase: BUKIO_SIGNING_PASSPHRASE, else an interactive prompt
 * (hidden input). Non-interactive shells without the env var fail. */
async function readPassphrase(actor) {
  const env = process.env.BUKIO_SIGNING_PASSPHRASE;
  if (env !== undefined && env !== '') return env;
  if (!process.stdin.isTTY) {
    throw actorCliError('PASSPHRASE_REQUIRED',
      `a passphrase is required for ${actor} — set BUKIO_SIGNING_PASSPHRASE or run interactively`);
  }
  return new Promise((resolve) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout, terminal: true });
    const original = rl._writeToOutput;
    if (typeof original === 'function') {
      rl._writeToOutput = (s) => {
        original.call(rl, s.includes('passphrase') ? s : '*');
      };
    }
    rl.question(`passphrase for ${actor}: `, (answer) => {
      rl.close();
      resolve(answer);
    });
  });
}

function parseTtlHours(value) {
  if (value === undefined) return DEFAULT_TTL_HOURS;
  const n = Number(value);
  if (!Number.isInteger(n) || n < 1 || n > MAX_TTL_HOURS) {
    throw actorCliError('INVALID_TTL', `--ttl-hours must be an integer 1–${MAX_TTL_HOURS}, got '${value}'`);
  }
  return n;
}

export function make(program) {
  const actor = program.command('actor').description('actor identity: keygen, register, list, revoke, enforce, unlock, lock, verify');

  actor
    .command('keygen')
    .description('generate an Ed25519 keypair — human keys are passphrase-encrypted, agent/system keys are plain files')
    .option('--force', 'replace an existing key file (rotation)')
    .option('--dry-run', 'show the plan without writing')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const who = ctx.actor;
        if (!isValidActor(who)) throw actorCliError('INVALID_ACTOR', `'${who}' is not a valid '<role>:<name>' actor`);
        const file = keyFilePath(who);
        const exists = existsSync(file);
        if (!ctx.dryRun && exists && !opts.force) {
          throw actorCliError('KEY_ALREADY_EXISTS', `key file ${file} exists — pass --force to replace it (rotation)`);
        }
        const isHuman = who.startsWith('human:');
        const passphrase = isHuman ? await readPassphrase(who) : null;
        const { publicKey, privateKey, keyid } = generateKeyPair(passphrase ? { passphrase } : {});
        const data = {
          actor: who, keyid, encrypted: Boolean(passphrase), keyFile: file,
          ...(ctx.dryRun ? { dryRun: true, wouldOverwrite: exists } : {}),
          ...(!ctx.dryRun ? { publicKey } : {}),
        };
        if (ctx.dryRun) {
          output(ctx, data, (d) => {
            console.log(`plan: keygen ${d.actor}`);
            console.log(`  keyid:   ${d.keyid}`);
            console.log(`  keyFile: ${d.keyFile} (${d.encrypted ? 'passphrase-encrypted' : 'plain'})`);
            if (d.wouldOverwrite) console.log('  note: key file exists — will be replaced');
            console.log('(dry run — nothing written)');
          });
          return;
        }
        writePrivateKeyFile(file, privateKey);
        output(ctx, data, (d) => {
          console.log(`key generated: ${d.actor}`);
          console.log(`  keyid:   ${d.keyid}`);
          console.log(`  keyFile: ${d.keyFile} (${d.encrypted ? 'passphrase-encrypted' : 'plain'})`);
          console.log(`  run 'bukio actor register --actor ${d.actor}' to enrol it in this company's DB`);
        });
      } catch (err) {
        fail(ctx, err);
      }
    });

  actor
    .command('register')
    .description("enrol the actor's local key into the current company's DB")
    .option('--dry-run', 'show the plan without writing')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const who = ctx.actor;
        const file = keyFilePath(who);
        if (!existsSync(file)) {
          throw actorCliError('KEY_NOT_FOUND', `no key file for ${who} at ${file} — run 'bukio actor keygen' first`);
        }
        const pem = readFileSync(file, 'utf8');
        const isHuman = who.startsWith('human:');
        const passphrase = isHuman && isEncrypted(pem) ? await readPassphrase(who) : null;
        let publicKey;
        try {
          publicKey = publicKeyFromPrivate(pem, passphrase ? { passphrase } : {});
        } catch {
          throw actorCliError('PASSPHRASE_INVALID', `could not read the key for ${who} — wrong passphrase or corrupt key file`);
        }
        const keyid = keyidOf(publicKey);
        const db = ensureDb(ctx);
        try {
          const data = { actor: who, keyid, enrolled: !ctx.dryRun, ...(ctx.dryRun ? { dryRun: true } : {}) };
          if (ctx.dryRun) {
            output(ctx, data, (d) => {
              console.log(`plan: register ${d.actor}`);
              console.log(`  keyid: ${d.keyid}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          const row = enrolActor(db, { actor: who, keyid, publicKey });
          record(db, {
            actor: ctx.actor, action: 'actor.register', command: 'actor register',
            args: { actor: who, keyid }, outcome: 'ok',
          });
          output(ctx, {
            actor: who, keyid, enrolled: true,
            row: { actor: row.actor, keyid: row.keyid, enrolled_at: row.enrolled_at, revoked_at: row.revoked_at },
          }, (d) => {
            console.log(`enrolled ${d.actor} (keyid ${d.keyid}) in this company's registry`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  actor
    .command('list')
    .description("list enrolled actors in the current company's DB")
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const rows = db.prepare(
            'SELECT actor, keyid, enrolled_at, revoked_at, revoked_reason FROM actor_keys ORDER BY actor',
          ).all();
          const actors = rows.map((r) => ({ ...r, active: r.revoked_at === null }));
          output(ctx, { actors }, (d) => {
            if (d.actors.length === 0) {
              console.log('no enrolled actors — run `bukio actor keygen` + `bukio actor register`');
              return;
            }
            table(d.actors, [
              { key: 'actor', label: 'actor' },
              { key: 'keyid', label: 'keyid' },
              { key: 'enrolled_at', label: 'enrolled' },
              { key: 'active', label: 'active' },
              { key: 'revoked_reason', label: 'revoked reason' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  actor
    .command('revoke')
    .description("revoke an actor's key in the current company's DB (row retained for history)")
    .option('--reason <text>', 'revocation reason (required)')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const who = ctx.actor;
        const db = ensureDb(ctx);
        try {
          const data = { actor: who, reason: opts.reason, ...(ctx.dryRun ? { dryRun: true } : {}) };
          if (ctx.dryRun) {
            output(ctx, data, (d) => {
              console.log(`plan: revoke ${d.actor}`);
              if (d.reason) console.log(`  reason: ${d.reason}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          const row = revokeActor(db, { actor: who, reason: opts.reason });
          record(db, {
            actor: ctx.actor, action: 'actor.revoke', command: 'actor revoke',
            args: { actor: who, reason: opts.reason }, outcome: 'ok',
          });
          output(ctx, { actor: who, keyid: row.keyid, revoked_at: row.revoked_at, reason: opts.reason }, (d) => {
            console.log(`revoked ${d.actor} (keyid ${d.keyid}) — reason: ${d.reason}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  actor
    .command('enforce')
    .description('toggle signed-command enforcement for this company (per-company, audited)')
    .option('--on', 'enforce signed commands: unsigned/invalid signatures are refused')
    .option('--off', 'disable enforcement (recovery/debugging)')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        if (opts.on === opts.off) {
          throw actorCliError('INVALID_ENFORCE', 'pass exactly one of --on or --off');
        }
        const on = Boolean(opts.on);
        const db = ensureDb(ctx);
        try {
          const data = { enforce: on ? 'on' : 'off', ...(ctx.dryRun ? { dryRun: true } : {}) };
          if (ctx.dryRun) {
            output(ctx, data, (d) => {
              console.log(`plan: enforcement ${d.enforce}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          setEnforce(db, on);
          record(db, {
            actor: ctx.actor, action: 'actor.enforce', command: 'actor enforce',
            args: { enforce: on ? 'on' : 'off' }, outcome: 'ok',
          });
          output(ctx, { enforce: on ? 'on' : 'off' }, (d) => {
            console.log(`signed-command enforcement is now ${d.enforce} for this company`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  actor
    .command('unlock')
    .description('unlock a human key for this session (writes a short-lived session key)')
    .option('--ttl-hours <n>', `session validity in hours (default ${DEFAULT_TTL_HOURS}, max ${MAX_TTL_HOURS})`)
    .option('--dry-run', 'show the plan without writing')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const who = ctx.actor;
        if (!who.startsWith('human:')) {
          throw actorCliError('UNLOCK_NOT_APPLICABLE',
            'only human: keys are unlocked per session — agent/system keys sign automatically from their key file');
        }
        const ttl = parseTtlHours(opts.ttlHours);
        const file = keyFilePath(who);
        if (!existsSync(file)) {
          throw actorCliError('KEY_NOT_FOUND', `no key file for ${who} at ${file} — run 'bukio actor keygen' first`);
        }
        const pem = readFileSync(file, 'utf8');
        const passphrase = await readPassphrase(who);
        let decrypted;
        try {
          decrypted = decryptPrivateKey(pem, { passphrase });
        } catch {
          throw actorCliError('PASSPHRASE_INVALID', 'wrong passphrase');
        }
        const expiresAt = new Date(Date.now() + ttl * 3600_000).toISOString();
        const session = sessionFilePath(who);
        const data = { actor: who, sessionFile: session, ttlHours: ttl, expiresAt, ...(ctx.dryRun ? { dryRun: true } : {}) };
        if (ctx.dryRun) {
          output(ctx, data, (d) => {
            console.log(`plan: unlock ${d.actor}`);
            console.log(`  sessionFile: ${d.sessionFile}`);
            console.log(`  expiresAt:   ${d.expiresAt} (${d.ttlHours}h)`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        mkdirSync(path.dirname(session), { recursive: true, mode: 0o700 });
        writeFileSync(session, JSON.stringify({ keyPem: decrypted, expiresAt }), { mode: 0o600 });
        output(ctx, data, (d) => {
          console.log(`unlocked ${d.actor} — session key valid until ${d.expiresAt} (${d.ttlHours}h)`);
        });
      } catch (err) {
        fail(ctx, err);
      }
    });

  actor
    .command('lock')
    .description('clear the session key, if any')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const who = ctx.actor;
        const session = sessionFilePath(who);
        const removed = existsSync(session);
        const data = { actor: who, sessionFile: session, removed, ...(ctx.dryRun ? { dryRun: true } : {}) };
        if (ctx.dryRun) {
          output(ctx, data, (d) => {
            console.log(`plan: lock ${d.actor}`);
            console.log(`  sessionFile: ${d.sessionFile}${d.removed ? ' (exists — will be removed)' : ' (absent)'}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        if (removed) rmSync(session);
        output(ctx, data, (d) => {
          console.log(d.removed ? `locked ${d.actor} — session key removed` : `${d.actor} has no session key`);
        });
      } catch (err) {
        fail(ctx, err);
      }
    });

  actor
    .command('verify')
    .description("check an actor's key state against the current company's registry")
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const who = ctx.actor;
        const file = keyFilePath(who);
        const db = ensureDb(ctx);
        try {
          const row = getActorKey(db, who);
          const data = {
            actor: who,
            keyFile: file,
            keyFileExists: existsSync(file),
            keyid: row?.keyid ?? null,
            registered: Boolean(row),
            active: canAct(db, who),
            revoked_at: row?.revoked_at ?? null,
            revoked_reason: row?.revoked_reason ?? null,
          };
          output(ctx, data, (d) => {
            console.log(`actor:      ${d.actor}`);
            console.log(`keyFile:    ${d.keyFile} (${d.keyFileExists ? 'present' : 'missing'})`);
            console.log(`keyid:      ${d.keyid ?? '(not registered)'}`);
            console.log(`registered: ${d.registered}`);
            console.log(`active:     ${d.active}`);
            if (d.revoked_at) console.log(`revoked:    ${d.revoked_at} — ${d.revoked_reason}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
