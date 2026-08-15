/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Remote access: `bukio server start` serves ONE company DB over HTTP(S).
// Clients (people on phones/laptops, agents on VPSes) send signed command
// envelopes — the same Tier 0 bundles the local sign gate produces — and
// the server verifies them against the company registry BEFORE executing.
// The private signing key never leaves the client device; only the public
// key is enrolled (via a one-time token minted by `bukio server token`).
//
// Execution model: the server does NOT re-implement command dispatch. Once
// an envelope verifies (signature + timestamp window + nonce replay + authz),
// the server spawns the CLI itself as a child process with the signed argv
// and BUKIO_REMOTE_EXEC=1 + the verified signature bundle in the env. The
// child skips the sign gate (the server already ran it) and just executes —
// its audit rows pick up the REAL signature via setPendingSignature, so
// `audit verify` on the server validates remote commands exactly like local
// ones. Byte-identical output to local mode, zero command-logic duplication.
//
// Transport: plain HTTP by default (bind to a trusted network — localhost,
// a tailnet IP, or behind a TLS reverse proxy); optional native TLS with
// --tls-cert/--tls-key. Envelope signatures provide authentication and
// integrity; TLS (or a trusted network) provides confidentiality.
import http from 'node:http';
import https from 'node:https';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../core/db.js';
import { buildDigest } from '../core/canonical.js';
import { getEnforce, enrolActor } from '../core/actor-registry.js';
import { checkAuthz } from '../core/authz.js';
import { verifySignatureBundle, configDir, dbError, sanitizeArgv, makeCtx, output, fail } from './util.js';
import { record } from '../audit/index.js';

const BIN_PATH = fileURLToPath(new URL('../../bin/bukio.js', import.meta.url));
const TOKENS_FILE = 'server-tokens.json';
const MAX_BODY_BYTES = 10 * 1024 * 1024; // 10 MB envelope cap

/** Commands that make no sense over the wire (bootstrap / bridge / local fs). */
export const REMOTE_LOCAL_ONLY = new Set([
  'server start', 'server token', 'mcp', 'init', 'update',
  'actor keygen', 'actor unlock', 'actor lock',
]);

function serverError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

// --- enrolment token store ---------------------------------------------------
// One-time, TTL'd tokens minted by `server token <actor>` (an operator act on
// the server machine). Stored sha256-hashed — the token file is a hash table,
// so a leaked token file does not leak usable tokens. `/register` exchanges a
// token for an enrolled public key, exactly like `actor register` does locally
// — but the token replaces the operator-gated enforce-off/register/enforce-on
// dance for REMOTE first enrolment (the token IS the operator gate).

function tokensPath() {
  return path.join(configDir(), TOKENS_FILE);
}

function readTokens() {
  try {
    return JSON.parse(readFileSync(tokensPath(), 'utf8'));
  } catch {
    return {};
  }
}

function writeTokens(tokens) {
  mkdirSync(path.dirname(tokensPath()), { recursive: true, mode: 0o700 });
  writeFileSync(tokensPath(), JSON.stringify(tokens, null, 2), { mode: 0o600 });
}

/** Mint a one-time enrolment token for an actor. Returns the raw token. */
export function mintEnrolToken(actor, { ttlHours = 24 } = {}) {
  const token = crypto.randomBytes(32).toString('base64url');
  const hash = crypto.createHash('sha256').update(token).digest('hex');
  const tokens = readTokens();
  tokens[hash] = {
    actor,
    createdAt: new Date().toISOString(),
    expiresAt: new Date(Date.now() + ttlHours * 3600_000).toISOString(),
    usedAt: null,
  };
  writeTokens(tokens);
  return token;
}

/**
 * Redeem a token: verify it exists, matches the actor, is unexpired and
 * unused. Throws TOKEN_INVALID / TOKEN_EXPIRED / TOKEN_USED /
 * TOKEN_ACTOR_MISMATCH. The token is marked used (single-use).
 */
export function consumeEnrolToken(token, actor) {
  if (!token) throw serverError('TOKEN_INVALID', 'an enrolment token is required');
  const hash = crypto.createHash('sha256').update(token).digest('hex');
  const tokens = readTokens();
  const entry = tokens[hash];
  if (!entry) throw serverError('TOKEN_INVALID', 'unknown enrolment token');
  if (entry.actor !== actor) {
    throw serverError('TOKEN_ACTOR_MISMATCH', `token was minted for ${entry.actor}, not ${actor}`);
  }
  if (new Date(entry.expiresAt).getTime() < Date.now()) {
    throw serverError('TOKEN_EXPIRED', 'enrolment token has expired — ask the operator for a fresh one');
  }
  if (entry.usedAt) {
    throw serverError('TOKEN_USED', 'enrolment token was already used (single-use)');
  }
  entry.usedAt = new Date().toISOString();
  writeTokens(tokens);
}

// --- the /rpc executor -------------------------------------------------------

/** Map a gate outcome to an HTTP response (all verification refusals). */
function gateErrorResponse(res, code, message) {
  res.writeHead(401, { 'content-type': 'application/json' });
  res.end(JSON.stringify({ ok: false, error: { code, message } }));
}

/**
 * Verify an envelope against the company registry and, when it passes, run
 * it as a child CLI process. Shared by the HTTP handler and tests.
 *
 * @param {object} db - open company DB (the server's --db).
 * @param {object} envelope - { v, actor, cmd, args, ts, nonce, digest, sig, keyid }.
 * @returns {Promise<{ok: boolean, stdout: string, stderr: string, exitCode: number}>}
 */
export async function executeEnvelope(db, envelope) {
  const { actor, cmd, args = {}, ts, nonce, sig, keyid } = envelope;
  const v = envelope.v ?? 1;
  // 1. LOCAL_ONLY blacklist (defense in depth — the client refuses too).
  if (REMOTE_LOCAL_ONLY.has(cmd)) {
    throw serverError('LOCAL_ONLY', `'${cmd}' cannot run remotely — it is a local/operator command`);
  }
  // 2. The digest is recomputed from the received fields, so any tampering
  //    with cmd/args/ts/nonce breaks the signature check below.
  const digest = buildDigest({ v, actor, cmd, args, ts, nonce });
  // 3. No signature at all → mirror local record semantics: refuse under
  //    enforce (SIGNATURE_REQUIRED), run unsigned in record mode.
  if (!sig || !keyid) {
    const enforce = getEnforce(db) === 'on';
    if (enforce) throw serverError('SIGNATURE_REQUIRED', `no signature in envelope for ${actor} — the company enforces signed commands`);
    // unsigned record mode: pass an unsigned bundle to the child
    const result = await runChild(db, { ...envelope, digest, sigStatus: 'unsigned', argv: args.argv });
    return result;
  }
  // 4. Full Tier 0 gate (replay nonce, ±5 min window, registry check).
  const gate = verifySignatureBundle(db, { actor, digest, sig, keyid, ts, nonce, enforce: getEnforce(db) === 'on' });
  if (!gate.ok) throw serverError(gate.code, gateMessage(gate.code, actor));
  // 5. Tier 0.5 authz gate (fail closed on unmapped commands).
  checkAuthz(db, actor, cmd, args);
  // 6. cmd must match the argv it claims (a signed envelope cannot execute
  //    a different command than the one in its argv).
  const argv = args.argv;
  if (!Array.isArray(argv) || argv.length === 0) {
    throw serverError('INVALID_ENVELOPE', 'envelope args.argv (the command line) is missing');
  }
  const cmdWords = cmd.split(' ').length;
  if (argv.slice(0, cmdWords).join(' ') !== cmd) {
    throw serverError('CMD_MISMATCH', `envelope cmd '${cmd}' does not match its argv`);
  }
  // 7. Execute.
  const result = await runChild(db, { ...envelope, digest, sigStatus: gate.status, argv: args.argv });
  return result;
}

function gateMessage(code, actor) {
  const messages = {
    ACTOR_KEY_UNKNOWN: `actor ${actor} has no enrolled key in this company's DB — run 'bukio actor register --server <url> --token <t>' (mint a token with 'bukio server token ${actor}' on the server)`,
    ACTOR_KEY_REVOKED: `the key for ${actor} is revoked in this company's DB — rotate with 'bukio actor keygen --force' + re-register via a fresh token`,
    SIGNATURE_STALE: 'signature timestamp is outside the ±5 minute window (clock skew?)',
    NONCE_REUSED: 'signature nonce was already used — a replayed command is refused',
    SIGNATURE_INVALID: `signature does not verify against the enrolled key for ${actor}`,
  };
  return messages[code] ?? 'signature verification failed';
}

/** Spawn the CLI with the signed argv; the child skips its own sign gate
 * (BUKIO_REMOTE_EXEC) and records the verified signature on its audit rows. */
function runChild(db, { actor, cmd, args, ts, nonce, sig, keyid, digest, sigStatus, argv }) {
  return new Promise((resolve, reject) => {
    const bundle = {
      digestHash: digest,
      sigKeyid: keyid ?? null,
      sigNonce: nonce ?? null,
      sigTs: ts ?? null,
      sig: sig ?? null,
      sigStatus,
      signedArgs: args,
      signedCommand: cmd,
    };
    const env = {
      ...process.env,
      BUKIO_DB: db.name,
      BUKIO_REMOTE_EXEC: '1',
      BUKIO_REMOTE_SIG: JSON.stringify(bundle),
    };
    delete env.BUKIO_SERVER; // the child must never re-enter the remote branch
    // defense in depth: never trust the envelope's argv with transport flags —
    // a hostile client could otherwise redirect the child to another DB
    const safeArgv = sanitizeArgv(argv);
    const child = spawn(process.execPath, [BIN_PATH, ...safeArgv], { env });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (d) => { stdout += d; });
    child.stderr.on('data', (d) => { stderr += d; });
    child.on('error', (err) => reject(serverError('SERVER_EXEC', `failed to spawn CLI: ${err.message}`)));
    child.on('close', (code) => resolve({ ok: code === 0, stdout, stderr, exitCode: code ?? 1 }));
  });
}

// --- HTTP server -------------------------------------------------------------

function readJsonBody(req, res) {
  return new Promise((resolve, reject) => {
    const len = Number(req.headers['content-length'] ?? 0);
    if (len > MAX_BODY_BYTES) {
      reject(serverError('BODY_TOO_LARGE', `request body exceeds ${MAX_BODY_BYTES} bytes`));
      return;
    }
    let body = '';
    req.on('data', (d) => {
      body += d;
      if (body.length > MAX_BODY_BYTES) {
        reject(serverError('BODY_TOO_LARGE', `request body exceeds ${MAX_BODY_BYTES} bytes`));
        req.destroy();
      }
    });
    req.on('end', () => {
      try {
        resolve(JSON.parse(body));
      } catch {
        reject(serverError('BAD_JSON', 'request body is not valid JSON'));
      }
    });
    req.on('error', reject);
  });
}

function sendJson(res, status, payload) {
  res.writeHead(status, { 'content-type': 'application/json' });
  res.end(JSON.stringify(payload));
}

/** Create the server command: `server start` (daemon) + `server token` (mint). */
export function make(program) {
  const server = program
    .command('server')
    .description('remote access: serve a company DB over HTTP(S), mint enrolment tokens');

  server
    .command('start')
    .description("serve ONE company DB: signed command envelopes in, CLI output out (run with nohup/systemd; 'server start' is a bridge and needs no signature)")
    .option('--listen <host:port>', 'bind address (default 127.0.0.1:8787)', '127.0.0.1:8787')
    .option('--serve-db <path>', 'company DB to serve (default: BUKIO_DB or ~/.bukio/bukio.db)', process.env.BUKIO_DB || path.join(process.env.HOME || '.', '.bukio', 'bukio.db'))
    .option('--tls-cert <path>', 'TLS certificate (PEM) — enables HTTPS')
    .option('--tls-key <path>', 'TLS private key (PEM) — required with --tls-cert')
    .action(async (opts) => {
      try {
        if (!existsSync(opts.serveDb)) throw dbError('NO_DATABASE', `no database at ${opts.serveDb} — the server serves an existing company DB`);
        if (opts.tlsCert && !opts.tlsKey) throw serverError('TLS_KEY_REQUIRED', '--tls-key is required with --tls-cert');
        const db = openDb(opts.serveDb);
        const [host, portRaw] = opts.listen.split(':');
        const port = Number(portRaw);
        if (!host || !Number.isInteger(port) || port < 0 || port > 65535) {
          throw serverError('INVALID_LISTEN', `--listen must be '<host>:<port>', got '${opts.listen}'`);
        }

        const handler = (req, res) => {
          const route = `${req.method} ${req.url.split('?')[0]}`;
          if (route === 'GET /health') {
            sendJson(res, 200, { ok: true, data: { status: 'ok', db: opts.serveDb } });
            return;
          }
          if (route !== 'POST /rpc' && route !== 'POST /register') {
            sendJson(res, 404, { ok: false, error: { code: 'NOT_FOUND', message: `no route ${route}` } });
            return;
          }
          readJsonBody(req, res)
            .then(async (body) => {
              if (route === 'POST /rpc') {
                const result = await executeEnvelope(db, body);
                sendJson(res, 200, result);
              } else {
                const { actor, keyid, publicKey, token } = body ?? {};
                consumeEnrolToken(token, actor); // throws on bad/used/expired/mismatched
                const row = enrolActor(db, { actor, keyid, publicKey }); // throws ALREADY_ENROLLED etc.
                record(db, {
                  actor, action: 'actor.register', command: 'actor register',
                  args: { actor, keyid, remote: true }, outcome: 'ok',
                });
                sendJson(res, 200, { ok: true, data: { actor: row.actor, keyid: row.keyid, enrolled_at: row.enrolled_at, remote: true } });
              }
            })
            .catch((err) => {
              const code = err.code || 'ERROR';
              if (code === 'AUTHZ_DENIED') {
                sendJson(res, 403, { ok: false, error: { code, message: err.message } });
              } else if (code === 'BAD_JSON' || code === 'BODY_TOO_LARGE' || code === 'INVALID_ENVELOPE' || code === 'CMD_MISMATCH') {
                sendJson(res, 400, { ok: false, error: { code, message: err.message } });
              } else {
                sendJson(res, 401, { ok: false, error: { code, message: err.message } });
              }
            });
        };

        const srv = opts.tlsCert
          ? https.createServer({ cert: readFileSync(opts.tlsCert), key: readFileSync(opts.tlsKey) }, handler)
          : http.createServer(handler);
        srv.listen(port, host, () => {
          const actual = srv.address();
          const shown = `${actual.address}:${actual.port}`;
          console.log(`listening on ${shown}`);
          console.log(`serving company DB: ${opts.serveDb}`);
          if (opts.tlsCert) console.log('TLS enabled (HTTPS)');
        });
        const shutdown = () => {
          try { db.close(); } catch { /* already closed */ }
          srv.close(() => process.exit(0));
          setTimeout(() => process.exit(0), 2000).unref();
        };
        process.on('SIGINT', shutdown);
        process.on('SIGTERM', shutdown);
      } catch (err) {
        console.error(`error [${err.code || 'ERROR'}]: ${err.message}`);
        process.exit(1);
      }
    });

  server
    .command('token')
    .description("mint a one-time enrolment token for a remote actor (operator act on the server machine — run 'bukio actor register --server <url> --token <t>' on the client to redeem)")
    .argument('<actor>', "actor to enrol, e.g. 'agent:bartholomeus' or 'human:erik'")
    .option('--ttl-hours <n>', 'token lifetime in hours (default 24)', '24')
    .action((actorArg, opts, command) => {
      const ctx = makeCtx(command);
      try {
        const who = actorArg;
        const ttlHours = Number(opts.ttlHours);
        if (!Number.isInteger(ttlHours) || ttlHours < 1 || ttlHours > 24 * 30) {
          throw serverError('INVALID_TTL', `--ttl-hours must be a whole number of hours (1..720), got '${opts.ttlHours}'`);
        }
        const token = mintEnrolToken(who, { ttlHours });
        output(ctx, { actor: who, token, ttlHours }, (d) => {
          console.log(`enrolment token for ${d.actor} (single-use, ${d.ttlHours}h TTL):`);
          console.log(d.token);
          console.log(`redeem with: bukio actor register --server <url> --token ${d.token}`);
        });
      } catch (err) {
        fail(ctx, err);
      }
    });

  return server;
}
