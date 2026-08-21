/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Backwards-compatible shim: the label/unit tables moved to src/i18n/ (S1,
// owner decision 15 Aug 2026). Importers of this module keep working.

export { UNIT_CODES, LABELS, UNITS, label, unitLabel } from '../i18n/index.js';
