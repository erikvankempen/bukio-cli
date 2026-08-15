/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Canonical command digest for signed actor commands (Tier 0 of the actor
// authentication design): a deterministic sha256 over the stable JSON shape
// { v, actor, cmd, args, ts, nonce }. The digest is what the actor signs and
// what `audit verify` recomputes from the stored args to detect tampering.
import crypto from 'node:crypto';

// Identity / output-format options are not part of the operation's semantics:
// who signs (--actor), which key file was used (--sign-key) and the output
// format (--json) must not change what a command *does*, so they are excluded
// from the signed digest (documented decision in the Tier 0 plan).
const EXCLUDED_ARGS = new Set(['actor', 'signKey', 'json']);

/**
 * Deterministic JSON: object keys sorted recursively (arrays keep their
 * elements, sorted by canonical string so order noise in e.g. postings
 * lists does not change the digest).
 *
 * @param {*} value
 * @returns {string} compact JSON with sorted keys.
 */
export function canonicalJson(value) {
  return JSON.stringify(sort(value));
}

function sort(value) {
  if (Array.isArray(value)) {
    return value.map(sort).sort((a, b) => {
      const ja = JSON.stringify(a);
      const jb = JSON.stringify(b);
      return ja < jb ? -1 : ja > jb ? 1 : 0;
    });
  }
  if (value && typeof value === 'object') {
    const out = {};
    for (const key of Object.keys(value).sort()) out[key] = sort(value[key]);
    return out;
  }
  return value;
}

/**
 * Build the sha256 digest (hex) of the canonical command shape.
 *
 * @param {object} input
 * @param {number} [input.v=1] - canonical format version.
 * @param {string} input.actor - declared actor, e.g. 'agent:bartholomeus'.
 * @param {string} input.cmd - command name, e.g. 'entry add'.
 * @param {object} [input.args={}] - commander options + positionals;
 *   'actor', 'signKey' and 'json' are excluded (see EXCLUDED_ARGS).
 * @param {string} input.ts - ISO 8601 UTC timestamp.
 * @param {string} input.nonce - one-time random token.
 * @returns {string} 64-hex-char sha256 digest.
 */
export function buildDigest({ v = 1, actor, cmd, args = {}, ts, nonce } = {}) {
  const cleanArgs = {};
  for (const [key, val] of Object.entries(args)) {
    if (!EXCLUDED_ARGS.has(key)) cleanArgs[key] = val;
  }
  const payload = { v, actor, cmd, args: cleanArgs, ts, nonce };
  return crypto.createHash('sha256').update(canonicalJson(payload), 'utf8').digest('hex');
}
