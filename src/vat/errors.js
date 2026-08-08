/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

export function vatError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}
