/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Remote access e2e: a `server start` daemon on an ephemeral port, one
// company DB, and CLI clients driving it over --server exactly as a human
// or agent on another device would. Covers: token minting + remote
// enrolment, remote read/mutation with audit-verifyable signatures, replay
// refusal, tamper detection, enforcement, authz, LOCAL_ONLY refusals,
// dry-run parity, human-output parity, same-device operation and the
// health endpoint. All through the real bin/bukio.js (like
// company-simulation), plus raw fetch() for the negative envelope tests.
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, rmSync, readFileSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import crypto from 'node:crypto';
import { buildDigest } from '../src/core/canonical.js';
import { keyidOf, publicKeyFromPrivate, sign } from '../src/core/sign.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

let dir;
let cfgDir;
let dbPath;
let serverUrl;
let serverProc;

/** Run a LOCAL CLI command (against the shared config dir). */
function local(args, { expectFail = false, env = {} } = {}) {
  const e = { ...process.env, BUKIO_CONFIG_DIR: cfgDir, BUKIO_DB: dbPath, ...env };
  try {
    const stdout = execFileSync(process.execPath, [BIN, ...args], { env: e, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    if (expectFail) return { code: err.status, out: JSON.parse(err.stdout), err: err.stderr };
    throw new Error(`local command failed (${args.join(' ')})` + (err.stdout ? `\nstdout: ${err.stdout.slice(0, 600)}` : '') + (err.stderr ? `\nstderr: ${err.stderr.slice(0, 400)}` : ''));
  }
}

/** Run a REMOTE CLI command against the test server. */
function remote(args, { expectFail = false } = {}) {
  const e = { ...process.env, BUKIO_CONFIG_DIR: cfgDir, BUKIO_ACTOR: 'agent:remote' };
  try {
    const stdout = execFileSync(process.execPath, [BIN, '--server', serverUrl, ...args], { env: e, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    if (expectFail) return { code: err.status, out: JSON.parse(err.stdout), err: err.stderr };
    throw new Error(`remote command failed (${args.join(' ')})` + (err.stdout ? `\nstdout: ${err.stdout.slice(0, 600)}` : '') + (err.stderr ? `\nstderr: ${err.stderr.slice(0, 400)}` : ''));
  }
}

/** Mint a token on the server side (operator act) and return it. */
function mintToken(actor, ttl = '24') {
  const out = execFileSync(process.execPath, [BIN, 'server', 'token', actor, '--actor', actor, '--ttl-hours', ttl], {
    env: { ...process.env, BUKIO_CONFIG_DIR: cfgDir },
    encoding: 'utf8',
  });
  const m = out.match(/^[A-Za-z0-9_-]{40,}$/m);
  assert.ok(m, `expected a token in output:\n${out}`);
  return m[0];
}

/**
 * Build a signed envelope for 'report trial-balance' with a given actor's
 * CLIENT key file. mutate(args) lets a test corrupt the signed payload
 * BEFORE signing (tamper tests) — or sign cleanly.
 */
function envelopeFor(actor, { mutate = null, dropSignature = false } = {}) {
  const pem = readFileSync(path.join(cfgDir, 'keys', `${actor.replace(':', '-')}.key`), 'utf8');
  const args = { argv: ['report', 'trial-balance', '--actor', actor, '--json'] };
  if (mutate) mutate(args);
  const ts = new Date().toISOString();
  const nonce = crypto.randomUUID();
  const digest = buildDigest({ v: 1, actor, cmd: 'report trial-balance', args, ts, nonce });
  const env = { v: 1, actor, cmd: 'report trial-balance', args, ts, nonce, digest, sig: null, keyid: null };
  if (!dropSignature) {
    env.sig = sign(digest, pem);
    env.keyid = keyidOf(publicKeyFromPrivate(pem));
  }
  return env;
}

async function postEnvelope(envelope) {
  return fetch(`${serverUrl}/rpc`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(envelope) });
}

before(async () => {
  dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-remote-'));
  cfgDir = path.join(dir, 'cfg');
  dbPath = path.join(dir, 'company.db');
  // server machine state: company DB + config dir (tokens, nonces)
  local(['init', '--name', 'Remote Test BV', '--actor', 'agent:op', '--json'], { env: {} });
  // the operator must be enrolled locally so `actor enforce` / `roles grant`
  // (both signed commands) can run against the server DB
  local(['actor', 'keygen', '--actor', 'agent:op', '--json']);
  local(['actor', 'register', '--actor', 'agent:op', '--json']);
  // the server daemon needs an actor identity to pass the gate, but is
  // signing-exempt (a bridge, like mcp)
  serverProc = spawn(process.execPath, [BIN, 'server', 'start', '--listen', '127.0.0.1:0', '--serve-db', dbPath, '--actor', 'agent:op'], {
    env: { ...process.env, BUKIO_CONFIG_DIR: cfgDir },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  await new Promise((resolve, reject) => {
    let out = '';
    const onData = (d) => {
      out += d;
      const m = out.match(/listening on (\S+)/);
      if (m) {
        serverUrl = `http://${m[1]}`;
        cleanup();
        resolve();
      }
    };
    const cleanup = () => {
      serverProc.stdout.off('data', onData);
      serverProc.stderr.off('data', onData);
    };
    serverProc.stdout.on('data', onData);
    serverProc.stderr.on('data', onData);
    serverProc.on('exit', (code) => {
      cleanup();
      reject(new Error(`server exited early (code ${code}):\n${out}`));
    });
    setTimeout(() => { cleanup(); reject(new Error(`server did not report a port:\n${out}`)); }, 5000).unref();
  });
});

after(() => {
  if (serverProc && !serverProc.killed) serverProc.kill('SIGTERM');
  rmSync(dir, { recursive: true, force: true });
});

// --- enrolment ---------------------------------------------------------------

test('server token: mints a single-use, actor-bound token (hashed at rest)', () => {
  const token = mintToken('agent:remote');
  assert.ok(token.length >= 40);
  const tokens = JSON.parse(readFileSync(path.join(cfgDir, 'server-tokens.json'), 'utf8'));
  const entries = Object.values(tokens);
  assert.equal(entries.length, 1);
  assert.equal(entries[0].actor, 'agent:remote');
  assert.ok(!entries[0].usedAt);
  // the raw token is NOT stored — only its sha256
  const hash = crypto.createHash('sha256').update(token).digest('hex');
  assert.ok(tokens[hash], 'token file must contain the sha256 of the minted token');
});

test('remote register: enrols a client-only key (private key never leaves the client)', () => {
  // client device: keygen (never on the server) + register via --server
  local(['actor', 'keygen', '--actor', 'agent:remote', '--json']);
  const token = mintToken('agent:remote');
  const r = local(['actor', 'register', '--server', serverUrl, '--token', token, '--actor', 'agent:remote', '--json']);
  assert.equal(r.out.ok, true, JSON.stringify(r.out));
  assert.equal(r.out.data.remote, true);
  assert.equal(r.out.data.actor, 'agent:remote');
  // the token is now consumed (single-use)
  const tokens = JSON.parse(readFileSync(path.join(cfgDir, 'server-tokens.json'), 'utf8'));
  const hash = crypto.createHash('sha256').update(token).digest('hex');
  assert.ok(tokens[hash]?.usedAt, 'the redeemed token must be single-use');
});

test('remote register: a used token is refused (TOKEN_USED)', () => {
  local(['actor', 'keygen', '--actor', 'agent:second', '--json']);
  const token = mintToken('agent:second');
  local(['actor', 'register', '--server', serverUrl, '--token', token, '--actor', 'agent:second', '--json']);
  const r = local(['actor', 'register', '--server', serverUrl, '--token', token, '--actor', 'agent:second', '--json'], { expectFail: true });
  assert.equal(r.out.error.code, 'TOKEN_USED');
});

test('remote register: an unknown / mismatched token is refused', () => {
  const bad = local(['actor', 'register', '--server', serverUrl, '--token', 'not-a-real-token-1234567890abcdef', '--actor', 'agent:second', '--json'], { expectFail: true });
  assert.equal(bad.out.error.code, 'TOKEN_INVALID');
  // a token minted for agent:second must not enrol a DIFFERENT actor; the
  // actor still needs a local key file (register reads the key first)
  local(['actor', 'keygen', '--actor', 'agent:other', '--json']);
  const token = mintToken('agent:second');
  const mismatch = local(['actor', 'register', '--server', serverUrl, '--token', token, '--actor', 'agent:other', '--json'], { expectFail: true });
  assert.equal(mismatch.out.error.code, 'TOKEN_ACTOR_MISMATCH');
});

test('remote register: --token is required with --server (TOKEN_REQUIRED)', () => {
  const r = local(['actor', 'register', '--server', serverUrl, '--actor', 'agent:second', '--json'], { expectFail: true });
  assert.equal(r.out.error.code, 'TOKEN_REQUIRED');
});

test('remote register: an expired token is refused (TOKEN_EXPIRED)', () => {
  local(['actor', 'keygen', '--actor', 'agent:expired', '--json']);
  const token = mintToken('agent:expired', '1');
  // expire it by backdating the stored entry
  const tokensPath = path.join(cfgDir, 'server-tokens.json');
  const tokens = JSON.parse(readFileSync(tokensPath, 'utf8'));
  for (const t of Object.values(tokens)) t.expiresAt = '2020-01-01T00:00:00.000Z';
  writeFileSync(tokensPath, JSON.stringify(tokens));
  const r = local(['actor', 'register', '--server', serverUrl, '--token', token, '--actor', 'agent:expired', '--json'], { expectFail: true });
  assert.equal(r.out.error.code, 'TOKEN_EXPIRED');
});

// --- remote execution --------------------------------------------------------

test('remote read: trial balance matches the local view (same device OK)', () => {
  const r = remote(['report', 'trial-balance', '--json']);
  assert.equal(r.out.ok, true);
  assert.equal(r.out.data.balanced, true);
  // same-device parity: the same command WITHOUT --server produces the same data
  const l = local(['report', 'trial-balance', '--actor', 'agent:remote', '--json']);
  assert.equal(l.out.ok, true);
  assert.equal(JSON.stringify(l.out.data), JSON.stringify(r.out.data));
});

test('remote mutation: posts an entry, the audit row carries the REAL signature', () => {
  const r = remote(['entry', 'add', '--date', '2026-08-11', '--desc', 'remote booking', '--postings', '1000:25000,8000:-25000', '--json']);
  assert.equal(r.out.ok, true, JSON.stringify(r.out));
  assert.equal(r.out.data.id, 1);
  // the server-side audit trail has a verified, replay-proof signature
  const audit = local(['audit', '--actor', 'agent:op', '--json']);
  const row = audit.out.data.entries.find((e) => e.action === 'entry.create');
  assert.ok(row, 'audit must contain the remote entry');
  assert.equal(row.sig_status, 'verified');
  assert.equal(row.sig_keyid.length, 32);
  assert.ok(row.sig);
  assert.ok(row.digest_hash);
  // audit verify on the SERVER db: the signature recomputes cleanly
  const v = local(['audit', 'verify', '--actor', 'agent:op', '--json']);
  assert.equal(v.out.data.summary.tampered, 0);
  assert.equal(v.out.data.summary.invalid_signature, 0);
});

test('remote mutation: dry-run parity (plan, no side effect)', () => {
  const before = local(['entry', 'list', '--actor', 'agent:op', '--json']).out.data.entries.length;
  const r = remote(['entry', 'add', '--date', '2026-08-12', '--desc', 'dry remote', '--postings', '1000:1000,8000:-1000', '--dry-run', '--json']);
  assert.equal(r.out.ok, true);
  assert.ok(r.out.data.dryRun);
  const after = local(['entry', 'list', '--actor', 'agent:op', '--json']).out.data.entries.length;
  assert.equal(after, before, 'dry-run must not mutate the books');
});

test('remote human output: byte-identical to local human output', () => {
  const e = { ...process.env, BUKIO_CONFIG_DIR: cfgDir, BUKIO_ACTOR: 'agent:remote' };
  const remoteText = execFileSync(process.execPath, [BIN, '--server', serverUrl, 'report', 'trial-balance'], { env: e, encoding: 'utf8' });
  const localText = execFileSync(process.execPath, [BIN, 'report', 'trial-balance'], { env: { ...e, BUKIO_DB: dbPath }, encoding: 'utf8' });
  assert.equal(remoteText, localText);
});

// --- negative envelope tests (raw HTTP, replayed/tampered) -------------------

test('replay: the SAME envelope twice is refused (NONCE_REUSED)', async () => {
  const envelope = envelopeFor('agent:remote');
  const first = await postEnvelope(envelope);
  assert.equal(first.status, 200);
  const second = await postEnvelope(envelope);
  const body = await second.json();
  assert.equal(body.ok, false);
  assert.equal(body.error.code, 'NONCE_REUSED');
});

test('tamper: changing the signed argv breaks the signature (SIGNATURE_INVALID)', async () => {
  // under enforce, a signature that does not match the transmitted args is
  // refused (record mode would tolerate it and log it as unsigned — the
  // same semantics as the local CLI)
  local(['actor', 'enforce', '--on', '--actor', 'agent:op', '--json']);
  try {
    // sign over args with a MUTATED argv, then restore the original argv in the
    // transmitted envelope → digest no longer matches what was signed
    const envelope = envelopeFor('agent:remote', {
      mutate: (args) => {
        args.argv = ['report', 'trial-balance', '--actor', 'agent:remote', '--json', '--mutated'];
      },
    });
    envelope.args.argv = ['report', 'trial-balance', '--actor', 'agent:remote', '--json'];
    const res = await postEnvelope(envelope);
    const body = await res.json();
    assert.equal(body.ok, false);
    assert.equal(body.error.code, 'SIGNATURE_INVALID');
  } finally {
    local(['actor', 'enforce', '--off', '--actor', 'agent:op', '--json']);
  }
});

test('enforcement: an unsigned envelope is refused under enforce (SIGNATURE_REQUIRED)', async () => {
  local(['actor', 'enforce', '--on', '--actor', 'agent:op', '--json']);
  try {
    const unsigned = envelopeFor('agent:remote', { dropSignature: true });
    const res = await postEnvelope(unsigned);
    const body = await res.json();
    assert.equal(body.ok, false);
    assert.equal(body.error.code, 'SIGNATURE_REQUIRED');
    // and a SIGNED envelope still works under enforce
    const signed = envelopeFor('agent:remote');
    const okRes = await postEnvelope(signed);
    assert.equal(okRes.status, 200);
  } finally {
    local(['actor', 'enforce', '--off', '--actor', 'agent:op', '--json']);
  }
});

test('authz: a readonly actor is refused a mutation (AUTHZ_DENIED)', async () => {
  // grant readonly to a fresh remote actor and turn authz on
  local(['actor', 'keygen', '--actor', 'agent:readonly', '--json']);
  const token = mintToken('agent:readonly');
  local(['actor', 'register', '--server', serverUrl, '--token', token, '--actor', 'agent:readonly', '--json']);
  local(['actor', 'roles', 'grant', 'readonly', '--for', 'agent:readonly', '--actor', 'agent:op', '--json']);
  local(['actor', 'authz', '--on', '--actor', 'agent:op', '--json']);
  try {
    const e = { ...process.env, BUKIO_CONFIG_DIR: cfgDir, BUKIO_ACTOR: 'agent:readonly' };
    let stdout = '';
    try {
      execFileSync(process.execPath, [BIN, '--server', serverUrl, 'entry', 'add', '--date', '2026-08-13', '--desc', 'nope', '--postings', '1000:1,8000:-1', '--json'], { env: e, encoding: 'utf8' });
    } catch (err) {
      stdout = err.stdout;
    }
    const body = JSON.parse(stdout);
    assert.equal(body.ok, false);
    assert.equal(body.error.code, 'AUTHZ_DENIED');
  } finally {
    local(['actor', 'authz', '--off', '--actor', 'agent:op', '--json']);
  }
});

// --- local-only commands -----------------------------------------------------

test('local-only commands refuse under --server (LOCAL_ONLY)', () => {
  for (const args of [
    ['actor', 'keygen', '--actor', 'agent:localonly'],
    ['mcp'],
    ['server', 'token', 'agent:x', '--actor', 'agent:x'],
    ['server', 'start', '--actor', 'agent:x'],
  ]) {
    const e = { ...process.env, BUKIO_CONFIG_DIR: cfgDir, BUKIO_ACTOR: 'agent:remote' };
    let body = null;
    try {
      execFileSync(process.execPath, [BIN, '--server', serverUrl, ...args, '--json'], { env: e, encoding: 'utf8' });
    } catch (err) {
      body = JSON.parse(err.stdout);
    }
    assert.ok(body, `expected LOCAL_ONLY for ${args.join(' ')}`);
    assert.equal(body.ok, false, `expected failure for ${args.join(' ')}`);
    assert.equal(body.error.code, 'LOCAL_ONLY', `expected LOCAL_ONLY for ${args.join(' ')}`);
  }
});

// --- health / misc -----------------------------------------------------------

test('health endpoint reports ok', async () => {
  // retry: under parallel-suite load a single connection attempt can fail
  // transiently while the daemon itself stays alive (the next test, unknown
  // route is 404, fetches the same server and passes)
  let res;
  for (let i = 0; i < 3; i++) {
    try {
      res = await fetch(`${serverUrl}/health`);
      break;
    } catch (e) {
      if (i === 2) throw e;
      await new Promise((r) => setTimeout(r, 250));
    }
  }
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.ok, true);
  assert.equal(body.data.status, 'ok');
});

test('unknown route is 404', async () => {
  const res = await fetch(`${serverUrl}/nope`);
  assert.equal(res.status, 404);
});

test('unreachable server: clean REMOTE_UNREACHABLE error', () => {
  const e = { ...process.env, BUKIO_CONFIG_DIR: cfgDir, BUKIO_ACTOR: 'agent:remote' };
  let body = null;
  try {
    execFileSync(process.execPath, [BIN, '--server', 'http://127.0.0.1:1', 'report', 'trial-balance', '--json'], { env: e, encoding: 'utf8' });
  } catch (err) {
    body = JSON.parse(err.stdout);
  }
  assert.ok(body);
  assert.equal(body.ok, false);
  assert.equal(body.error.code, 'REMOTE_UNREACHABLE');
});

test('server token rejects a bad --ttl-hours value', () => {
  let body = null;
  try {
    execFileSync(process.execPath, [BIN, 'server', 'token', 'agent:x', '--actor', 'agent:x', '--ttl-hours', 'abc', '--json'], {
      env: { ...process.env, BUKIO_CONFIG_DIR: cfgDir },
      encoding: 'utf8',
    });
  } catch (err) {
    body = JSON.parse(err.stdout);
  }
  assert.ok(body);
  assert.equal(body.ok, false);
  assert.equal(body.error.code, 'INVALID_TTL');
});

test('envelope can carry the --db of the CLIENT but the server DB is authoritative', async () => {
  // a signed envelope whose argv names a DIFFERENT (bogus) --db must still
  // execute against the server's company DB — sanitizeArgv strips it
  const pem = readFileSync(path.join(cfgDir, 'keys', 'agent-remote.key'), 'utf8');
  const args = { argv: ['report', 'trial-balance', '--actor', 'agent:remote', '--db', '/tmp/bogus.db', '--json'] };
  const ts = new Date().toISOString();
  const nonce = crypto.randomUUID();
  const digest = buildDigest({ v: 1, actor: 'agent:remote', cmd: 'report trial-balance', args, ts, nonce });
  const envelope = {
    v: 1, actor: 'agent:remote', cmd: 'report trial-balance', args, ts, nonce, digest,
    sig: sign(digest, pem), keyid: keyidOf(publicKeyFromPrivate(pem)),
  };
  const res = await postEnvelope(envelope);
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.ok, true);
  assert.equal(body.exitCode, 0);
});
