/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 *
 * The Rust port embeds src/profiles.json (include_str!) instead of porting 31
 * profile modules by hand. That makes the generator the only thing standing
 * between a profile edit and a stale port, so assert the file is exactly what
 * the JS profiles produce. This covers every field of every profile, which is
 * strictly more than the per-country assertions it replaces.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { getProfile } from '../src/jurisdictions/index.js';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');

test('src/profiles.json is in sync with the JS jurisdiction profiles', () => {
  const codes = readdirSync(path.join(root, 'src', 'jurisdictions'))
    .filter((f) => /^[a-z]{2}\.js$/.test(f))
    .map((f) => f.slice(0, 2).toUpperCase());
  assert.equal(codes.length, 31, 'thirty-one markets');
  const profiles = {};
  for (const c of codes) profiles[c] = getProfile(c);
  const want = `${JSON.stringify(profiles, null, 1)}\n`;
  const have = readFileSync(path.join(root, 'src', 'profiles.json'), 'utf8');
  assert.equal(have, want, 'stale port — run: node scripts/gen-profiles.mjs');
});
