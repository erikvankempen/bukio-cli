#!/usr/bin/env node
/**
 * bukio-cli — generate src/profiles.json from the JS jurisdiction profiles.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 *
 * The JS modules are the single source of truth; the Rust port embeds the
 * generated JSON (include_str!). Run after any profile change:
 *   node scripts/gen-profiles.mjs
 */
import { writeFileSync, mkdirSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { getProfile } from '../src/jurisdictions/index.js';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const dir = path.join(root, 'src', 'jurisdictions');
const codes = readdirSync(dir)
  .filter((f) => /^[a-z]{2}\.js$/.test(f))
  .map((f) => f.slice(0, 2).toUpperCase());
const profiles = {};
for (const c of codes) profiles[c] = getProfile(c);
mkdirSync(path.join(root, 'src'), { recursive: true });
writeFileSync(path.join(root, 'src', 'profiles.json'), JSON.stringify(profiles, null, 1) + '\n');
console.log(`generated src/profiles.json (${codes.length} jurisdictions: ${codes.join(' ')})`);
