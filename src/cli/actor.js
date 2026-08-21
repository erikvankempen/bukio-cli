/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio actor — Tier 0 key-bound actor identity: keygen/register/list/
// revoke/enforce/unlock/lock/verify. Keys live under the config dir
// (BUKIO_CONFIG_DIR or ~/.bukio): keys/<role>-<name>.key (0600, dir 0700),
// sessions/<role>-<name>.key for short-lived human unlocks. The registry
// itself is per-company (actor_keys table in the company DB).
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import {
  ensureDb, makeCtx, output, fail, table, configDir, keyFilePath, sessionFilePath, readSessionKey,
  withDb,
} from './util.js';
import {
  generateKeyPair, keyidOf, isEncrypted, publicKeyFromPrivate, decryptPrivateKey,
} from '../core/sign.js';
import { isValidActor } from '../core/actor.js';
import {
  enrolActor, revokeActor, getActorKey, setEnforce, getEnforce, canAct,
  setAuthz, getAuthz, grantRole, revokeRole, getRoles, hasRole, listRoleGrants,
} from '../core/actor-registry.js';
import {
  capabilityOf, canAct as canActCapability, sodWarnings, ROLES as VALID_ROLES,
} from '../core/authz.js';
import { record } from '../audit/index.js';

const DEFAULT_TTL_HOURS = 12;
const MAX_TTL_HOURS = 168; // 7 days

export function actorCliError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
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
    .description("enrol the actor's key: locally into the current company's DB, or remotely via '--server <url>' + '--token <t>' (mint the token with 'bukio server token' on the server machine)")
    .option('--dry-run', 'show the plan without writing')
    .option('--token <t>', 'one-time enrolment token (required with --server)')
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
        if (ctx.server) {
          // Remote enrolment: the one-time token IS the operator gate (it
          // replaces the local enforce-off/register/enforce-on dance).
          if (!opts.token) {
            throw actorCliError('TOKEN_REQUIRED', `remote registration needs --token <t> — mint one with 'bukio server token ${who}' on the server machine`);
          }
          if (ctx.dryRun) {
            output(ctx, { actor: who, keyid, server: ctx.server, dryRun: true }, (d) => {
              console.log(`plan: register ${d.actor} (keyid ${d.keyid}) at ${d.server}`);
              console.log('(dry run — nothing sent)');
            });
            return;
          }
          let res;
          try {
            res = await fetch(`${ctx.server.replace(/\/$/, '')}/register`, {
              method: 'POST',
              headers: { 'content-type': 'application/json' },
              body: JSON.stringify({ actor: who, keyid, publicKey, token: opts.token }),
            });
          } catch (err) {
            throw actorCliError('REMOTE_UNREACHABLE', `cannot reach ${ctx.server}: ${err.message}`);
          }
          const body = await res.json().catch(() => null);
          if (!res.ok || !body?.ok) {
            throw actorCliError(body?.error?.code ?? 'REMOTE_ERROR', body?.error?.message ?? `registration failed (HTTP ${res.status})`);
          }
          output(ctx, { ...body.data, server: ctx.server }, (d) => {
            console.log(`enrolled ${d.actor} (keyid ${d.keyid}) at ${d.server} — remote commands with '--server ${d.server}' will now verify`);
          });
          return;
        }
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
    .action((opts, command) => withDb(command, (ctx, db) => {
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
    }));

  actor
    .command('revoke')
    .description("revoke an actor's key in the current company's DB (row retained for history). Default: your own key. --target <who> is the OWNER-mediated kill of a compromised key (owner role required, regardless of authz mode).")
    .option('--reason <text>', 'revocation reason (required)')
    .option('--target <who>', "revoke ANOTHER actor's key — owner mediated (requires the owner role)")
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const who = opts.target ?? ctx.actor;
        if (!isValidActor(who)) throw actorCliError('INVALID_ACTOR', `'${who}' is not a valid '<role>:<name>' actor`);
        const db = ensureDb(ctx);
        try {
          const data = { actor: who, reason: opts.reason, ...(opts.target ? { revoked_by: ctx.actor } : {}), ...(ctx.dryRun ? { dryRun: true } : {}) };
          if (ctx.dryRun) {
            output(ctx, data, (d) => {
              console.log(`plan: revoke ${d.actor}${d.revoked_by ? ` (by ${d.revoked_by})` : ''}`);
              if (d.reason) console.log(`  reason: ${d.reason}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          const row = revokeActor(db, { actor: who, reason: opts.reason });
          record(db, {
            actor: ctx.actor, action: 'actor.revoke', command: 'actor revoke',
            args: { actor: who, reason: opts.reason, ...(opts.target ? { revoked_by: ctx.actor } : {}) }, outcome: 'ok',
          });
          output(ctx, { actor: who, keyid: row.keyid, revoked_at: row.revoked_at, reason: opts.reason, ...(opts.target ? { revoked_by: ctx.actor } : {}) }, (d) => {
            console.log(`revoked ${d.actor} (keyid ${d.keyid})${d.revoked_by ? ` — by ${d.revoked_by}` : ''} — reason: ${d.reason}`);
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
    .command('authz')
    .description("toggle per-actor authorizations (segregation of duties) for this company. --on implies signing enforcement and grants the flipper the owner role (bootstrap, D3); --off needs the owner role.")
    .option('--on', 'enable authorizations: every non-exempt command needs a role granting its capability (deny-by-default)')
    .option('--off', 'disable authorizations (owner only; signing enforcement stays on)')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        if (opts.on === opts.off) {
          throw actorCliError('INVALID_AUTHZ', 'pass exactly one of --on or --off');
        }
        const on = Boolean(opts.on);
        const db = ensureDb(ctx);
        try {
          const currentlyOn = getAuthz(db) === 'on';
          const data = {
            authz: on ? 'on' : 'off',
            ...(on && !currentlyOn ? { enforce: 'on (implied)', owner_granted: ctx.actor } : {}),
            ...(ctx.dryRun ? { dryRun: true } : {}),
          };
          if (ctx.dryRun) {
            output(ctx, data, (d) => {
              console.log(`plan: authorizations ${d.authz}`);
              if (d.owner_granted) {
                console.log(`  owner role will be granted to ${d.owner_granted} (bootstrap — the flipper becomes owner)`);
                console.log('  signing enforcement will be switched on (authz implies enforce)');
              }
              console.log('(dry run — nothing written)');
            });
            return;
          }
          if (on && !currentlyOn) {
            db.transaction(() => {
              setEnforce(db, true); // D1: authz implies enforce
              setAuthz(db, true);
              if (!hasRole(db, ctx.actor, 'owner')) {
                grantRole(db, { actor: ctx.actor, role: 'owner', grantedBy: ctx.actor }); // D3: flipper becomes owner
              }
            })();
            record(db, {
              actor: ctx.actor, action: 'actor.authz', command: 'actor authz',
              args: { authz: 'on', enforce: 'on', owner: ctx.actor }, outcome: 'ok',
            });
          } else {
            setAuthz(db, on);
            record(db, {
              actor: ctx.actor, action: 'actor.authz', command: 'actor authz',
              args: { authz: on ? 'on' : 'off' }, outcome: 'ok',
            });
          }
          output(ctx, {
            authz: getAuthz(db),
            enforce: getEnforce(db),
            ...(on && !currentlyOn ? { owner: ctx.actor } : {}),
          }, (d) => {
            console.log(`per-actor authorizations are now ${d.authz} for this company`);
            if (d.owner) {
              console.log(`  owner role granted to ${d.owner} (bootstrap)`);
              console.log('  signing enforcement switched on (authz implies enforce)');
            }
            console.log(`  signing enforcement: ${d.enforce}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  const roles = actor
    .command('roles')
    .description("role grants (segregation of duties): list your own roles, or --for <who> to see another's (owner only under authz)")
    .option('--for <who>', "show another actor's roles (owner only under authz)", undefined)
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const who = opts.for ?? ctx.actor;
        if (!isValidActor(who)) throw actorCliError('INVALID_ACTOR', `'${who}' is not a valid '<role>:<name>' actor`);
        const db = ensureDb(ctx);
        try {
          const roles = getRoles(db, who);
          output(ctx, { actor: who, roles }, (d) => {
            if (d.roles.length === 0) {
              console.log(`${d.actor} holds no roles (deny-by-default under authz: only self-service checks work)`);
              return;
            }
            console.log(`${d.actor}: ${d.roles.join(', ')}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  roles
    .command('grant')
    .description(`grant a role to an actor (owner only under authz). Warns on segregation-of-duties conflicts — the warning is soft, the owner decides. Roles: ${VALID_ROLES.join('|')}`)
    .argument('<role>', `one of ${VALID_ROLES.join('|')}`)
    .option('--for <who>', "actor to grant the role to, e.g. 'agent:invoicing' (required)")
    .option('--dry-run', 'show the plan without writing')
    .action((role, opts, command) => {
      const ctx = makeCtx(command);
      try {
        // NOTE: `--for` is declared on the parent `roles` command — the
        // identity flag `--actor` is the root option and cannot be
        // re-declared here (commander binds the value to the root and the
        // signing identity would be corrupted)
        const target = command.parent.opts().for;
        if (!target) {
          throw actorCliError('INVALID_ACTOR', "missing required option '--for <who>' — the actor to grant the role to");
        }
        if (!VALID_ROLES.includes(role)) {
          throw actorCliError('INVALID_ROLE', `'${role}' is not a role — use one of ${VALID_ROLES.join('|')}`);
        }
        if (!isValidActor(target)) throw actorCliError('INVALID_ACTOR', `'${target}' is not a valid '<role>:<name>' actor`);
        const db = ensureDb(ctx);
        try {
          const after = [...new Set([...getRoles(db, target), role])].sort();
          const warnings = sodWarnings(after);
          const data = { actor: target, role, roles: after, warnings, ...(ctx.dryRun ? { dryRun: true } : {}) };
          if (ctx.dryRun) {
            output(ctx, data, (d) => {
              console.log(`plan: grant ${d.role} to ${d.actor}`);
              console.log(`  resulting roles: ${d.roles.join(', ') || '(none)'}`);
              for (const w of d.warnings) console.log(`  ⚠ SoD: ${w}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          grantRole(db, { actor: target, role, grantedBy: ctx.actor });
          record(db, {
            actor: ctx.actor, action: 'actor.roles.grant', command: 'actor roles grant',
            args: { role, actor: target }, outcome: 'ok',
          });
          output(ctx, data, (d) => {
            console.log(`granted ${d.role} to ${d.actor} (roles: ${d.roles.join(', ') || 'none'})`);
            for (const w of d.warnings) console.log(`  ⚠ SoD: ${w}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  roles
    .command('revoke')
    .description('remove a role from an actor (owner only under authz). The LAST owner can never be revoked.')
    .argument('<role>', `one of ${VALID_ROLES.join('|')}`)
    .option('--for <who>', 'actor to revoke the role from (required)')
    .option('--dry-run', 'show the plan without writing')
    .action((role, opts, command) => {
      const ctx = makeCtx(command);
      try {
        const target = command.parent.opts().for;
        if (!target) {
          throw actorCliError('INVALID_ACTOR', "missing required option '--for <who>' — the actor to revoke the role from");
        }
        if (!VALID_ROLES.includes(role)) {
          throw actorCliError('INVALID_ROLE', `'${role}' is not a role — use one of ${VALID_ROLES.join('|')}`);
        }
        if (!isValidActor(target)) throw actorCliError('INVALID_ACTOR', `'${target}' is not a valid '<role>:<name>' actor`);
        const db = ensureDb(ctx);
        try {
          const after = getRoles(db, target).filter((r) => r !== role).sort();
          const data = { actor: target, role, roles: after, ...(ctx.dryRun ? { dryRun: true } : {}) };
          if (ctx.dryRun) {
            output(ctx, data, (d) => {
              console.log(`plan: revoke ${d.role} from ${d.actor}`);
              console.log(`  resulting roles: ${d.roles.join(', ') || '(none)'}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          revokeRole(db, { actor: target, role });
          record(db, {
            actor: ctx.actor, action: 'actor.roles.revoke', command: 'actor roles revoke',
            args: { role, actor: target }, outcome: 'ok',
          });
          output(ctx, data, (d) => {
            console.log(`revoked ${d.role} from ${d.actor} (roles: ${d.roles.join(', ') || 'none'})`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  actor
    .command('can')
    .description("capability check: is the command allowed for the actor? Self-service for your own checks; --for <who> is owner only under authz. Accepts 'entry add --post' or 'mcp:entry_add'.")
    .argument('<command>', "command path, e.g. 'entry add', 'entry add --post', 'vat file' or 'mcp:entry_add'")
    .option('--for <who>', 'check another actor (owner only under authz)', undefined)
    .action((commandArg, opts, command) => {
      const ctx = makeCtx(command);
      try {
        const who = opts.for ?? ctx.actor;
        const tokens = String(commandArg).trim().split(/\s+/);
        if (tokens[0] === 'bukio') tokens.shift();
        const flags = tokens.filter((t) => t.startsWith('-'));
        const words = tokens.filter((t) => !t.startsWith('-'));
        const path = words.join(' ');
        const args = { positionals: [commandArg] };
        if (flags.includes('--post')) args.post = true;
        if (flags.includes('--target')) args.target = true;
        const capability = capabilityOf(path, args);
        const db = ensureDb(ctx);
        try {
          const allowed = capability ? canActCapability(db, who, capability) : false;
          const roles = getRoles(db, who);
          output(ctx, {
            actor: who, command: commandArg, capability, allowed, roles,
            ...(capability && !allowed ? { denied_reason: `no capability '${capability}'` } : {}),
            ...(!capability ? { denied_reason: 'no capability mapping (fail closed)' } : {}),
          }, (d) => {
            if (d.allowed) {
              console.log(`ok — ${d.actor} can run '${d.command}' (capability: ${d.capability}, roles: ${d.roles.join(', ') || 'none'})`);
            } else {
              console.log(`AUTHZ_DENIED — ${d.actor} cannot run '${d.command}': ${d.denied_reason}`);
              if (d.roles.length) console.log(`  roles: ${d.roles.join(', ')}`);
              console.log('  ask the owner to grant a role that carries this capability');
            }
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  actor
    .command('who-can')
    .description("which actors can run a command (segregation-of-duties review; owner only under authz)")
    .argument('<command>', "command path, e.g. 'entry post' or 'payments batch create'")
    .action((commandArg, opts, command) => {
      const ctx = makeCtx(command);
      try {
        const tokens = String(commandArg).trim().split(/\s+/);
        if (tokens[0] === 'bukio') tokens.shift();
        const flags = tokens.filter((t) => t.startsWith('-'));
        const words = tokens.filter((t) => !t.startsWith('-'));
        const path = words.join(' ');
        const args = { positionals: [commandArg] };
        if (flags.includes('--post')) args.post = true;
        const capability = capabilityOf(path, args);
        const db = ensureDb(ctx);
        try {
          // every actor that holds a role, plus every enrolled actor
          const names = new Set([...listRoleGrants(db).map((r) => r.actor)]);
          for (const a of db.prepare('SELECT DISTINCT actor FROM actor_keys').all()) names.add(a.actor);
          const actors = [...names].sort().map((actor) => {
            const roles = getRoles(db, actor);
            return { actor, roles, allowed: capability ? canActCapability(db, actor, capability) : false };
          });
          output(ctx, { command: commandArg, capability, actors }, (d) => {
            console.log(`'${d.command}' → capability: ${d.capability ?? '(none — fail closed)'}`);
            if (d.actors.length === 0) {
              console.log('  no actors hold roles or keys in this company');
              return;
            }
            for (const a of d.actors) {
              console.log(`  ${a.allowed ? '✔' : '✘'} ${a.actor} (${a.roles.join(', ') || 'no roles'})`);
            }
          });
        } finally {
          db.close();
        }
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
