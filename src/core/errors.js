/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Shared error factory. Every domain module previously carried its own 3-line
// `xyzError(code, message)` copy; they all produce the same shape (Error with
// a .code property), so one factory serves all of them. Modules keep their own
// code strings — only the boilerplate goes away.

export function makeError(code, message, details = null) {
  const e = new Error(message);
  e.code = code;
  if (details) e.details = details;
  return e;
}
