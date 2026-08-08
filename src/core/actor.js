/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Actor identity: '<role>:<name>' — required on every CLI and MCP action so
// the audit trail always names who acted (agent:bartholomeus, human:erik).
// A bare 'human' or 'agent' without a name is rejected: anonymous actors
// defeat the purpose of an audited ledger.

const ACTOR_RE = /^(agent|human|system):[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;

export function isValidActor(actor) {
  return typeof actor === 'string' && ACTOR_RE.test(actor);
}

/** Return {code, message} for a missing/invalid actor, or null when valid. */
export function actorError(actor) {
  if (!actor) {
    return {
      code: 'ACTOR_REQUIRED',
      message: "missing actor — pass --actor '<role>:<name>' (e.g. agent:bartholomeus, human:erik) or set BUKIO_ACTOR",
    };
  }
  if (!isValidActor(actor)) {
    return {
      code: 'INVALID_ACTOR',
      message: `invalid actor '${actor}' — must be '<role>:<name>' (agent|human|system), e.g. agent:bartholomeus or human:erik`,
    };
  }
  return null;
}
