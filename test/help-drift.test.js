/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 *
 * The Rust port embeds src/help.json (include_str!) instead of re-implementing
 * commander's usage rendering for 160 command paths. That makes
 * scripts/help-tree.mjs the only thing between a CLI change and a stale port, so
 * assert the file is exactly what the JS CLI's own help produces. A renamed or
 * added command fails here rather than silently returning the wrong usage text.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { captureHelp } from '../scripts/help-tree.mjs';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');

test('src/help.json is in sync with the JS CLI help output', () => {
  const want = `${JSON.stringify(captureHelp(), null, 1)}\n`;
  const have = readFileSync(path.join(root, 'src', 'help.json'), 'utf8');
  assert.equal(
    have,
    want,
    'stale port — run: node scripts/help-tree.mjs',
  );
});
