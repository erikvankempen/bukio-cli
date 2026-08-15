/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Remote client: `--server <url>` (or BUKIO_SERVER env) turns ANY bukio
// command into a signed envelope POSTed to a `bukio server start` daemon.
// The private key stays on this device; only the signed digest travels.
// The server verifies the envelope against its company registry and runs
// the command; the response body is byte-identical to running locally.
//
// Envelope shape (POST /rpc):
//   { v: 1, actor, cmd, args, ts, nonce, digest, sig, keyid }
//   args includes the sanitized argv as args.argv, so the digest covers
//   BOTH the parsed options AND the exact command line that will execute —
//   a client cannot sign one thing and execute another.
import { buildDigest } from '../core/canonical.js';
import { sign } from '../core/sign.js';
import { resolveSigningKey, buildSignedArgs, commandPathOf, sanitizeArgv } from './util.js';
import { REMOTE_LOCAL_ONLY } from './server.js';
import crypto from 'node:crypto';

/**
 * Build the signed envelope for a commander action: parse the options,
 * sanitize the raw argv, digest everything, sign with the local key.
 *
 * @param {object} ctx - { actor } from makeCtx.
 * @param {object} command - the commander action command.
 * @param {string[]} rawArgv - the original argv (minus node+script).
 * @returns {object} the envelope to POST to /rpc.
 */
export function buildRemoteEnvelope(ctx, command, rawArgv) {
  const opts = command.optsWithGlobals();
  const cmd = commandPathOf(command);
  const argv = sanitizeArgv(rawArgv);
  // the child must know who acted even when the actor came from BUKIO_ACTOR
  if (!argv.includes('--actor') && ctx.actor) argv.push('--actor', ctx.actor);
  const args = buildSignedArgs(opts, command, ctx.actor, ['db', 'server']);
  args.argv = argv;
  const ts = new Date().toISOString();
  const nonce = crypto.randomUUID();
  const digest = buildDigest({ v: 1, actor: ctx.actor, cmd, args, ts, nonce });
  const key = resolveSigningKey(ctx.actor, opts.signKey ?? null, false);
  const envelope = { v: 1, actor: ctx.actor, cmd, args, ts, nonce, digest, sig: null, keyid: null };
  if (key) {
    envelope.sig = sign(digest, key.keyPem);
    envelope.keyid = key.keyid;
  }
  return envelope;
}

/**
 * POST an envelope to the server and return the parsed response.
 * Throws REMOTE_UNREACHABLE / REMOTE_ERROR on transport or gate failures
 * (caller formats with fail()).
 *
 * @param {string} server - base URL, e.g. 'http://127.0.0.1:8787'.
 * @param {object} envelope - from buildRemoteEnvelope.
 * @returns {object} the executed-command result { ok, stdout, stderr, exitCode }.
 */
export async function postRemote(server, envelope) {
  let res;
  try {
    res = await fetch(`${server.replace(/\/$/, '')}/rpc`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(envelope),
    });
  } catch (err) {
    const e = new Error(`cannot reach ${server}: ${err.message}`);
    e.code = 'REMOTE_UNREACHABLE';
    throw e;
  }
  let body;
  try {
    body = await res.json();
  } catch {
    const e = new Error(`remote server returned non-JSON (HTTP ${res.status})`);
    e.code = 'REMOTE_ERROR';
    throw e;
  }
  if (!res.ok || body.error) {
    const e = new Error(body?.error?.message ?? `remote command failed (HTTP ${res.status})`);
    e.code = body?.error?.code ?? 'REMOTE_ERROR';
    throw e;
  }
  return body;
}

/**
 * Remote-mode preAction: intercept a command bound for a server. Refuses
 * LOCAL_ONLY commands, lets `actor register` through (its action does the
 * /register POST), and otherwise signs + POSTs + emits the response.
 *
 * @param {object} ctx - { json, actor } from makeCtx.
 * @param {object} command - the commander action command.
 * @param {string[]} rawArgv - the original argv.
 * @param {string} server - base URL.
 * @returns {Promise<number>} exit code (0 = ok) — caller exits with it.
 */
export async function remotePreAction(ctx, command, rawArgv, server) {
  const cmd = commandPathOf(command);
  if (REMOTE_LOCAL_ONLY.has(cmd)) {
    const e = new Error(`'${cmd}' cannot run with --server — it is a local/operator command`);
    e.code = 'LOCAL_ONLY';
    throw e;
  }
  if (cmd === 'actor register') return; // the action performs the /register POST
  const envelope = buildRemoteEnvelope(ctx, command, rawArgv);
  const result = await postRemote(server, envelope);
  process.stdout.write(result.stdout);
  process.stderr.write(result.stderr);
  return result.exitCode === 0 ? 0 : 1;
}
