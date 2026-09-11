#!/usr/bin/env node
/**
 * bukio-cli — generate src/help.json from the JS CLI's own help output.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 *
 * The JS CLI is built on commander, which renders the usage/help text for free.
 * The Rust port has a hand-rolled dispatch with no such layer, so `bukio bank
 * --help` answered UNKNOWN_COMMAND and `bukio --help` listed only the global
 * flags. Rather than hand-maintain a second copy of the command tree — which
 * would drift the moment a command is renamed — capture commander's own output
 * once and embed it (include_str!), the same arrangement as profiles.json.
 *
 * The walk is driven by the help text itself: any "Commands:" section names the
 * children to visit, so a new subcommand appears here without editing this file.
 *
 *   node scripts/help-tree.mjs        # regenerate src/help.json
 */
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const MAX_DEPTH = 4;

/** The child command names listed under "Commands:" in a help text. */
function childrenOf(text) {
  const lines = text.split('\n');
  const i = lines.findIndex((l) => l.trim() === 'Commands:');
  if (i === -1) return [];
  const out = [];
  for (const line of lines.slice(i + 1)) {
    if (!line.trim()) break;
    // "  add [options]   register a bank account" — a wrapped description line
    // is indented further and has no [options]/<arg> before the gap
    const m = /^ {2,4}(\S+)(\s+(\[options\]|<[^>]*>))?\s{2,}\S/.exec(line);
    if (m) out.push(m[1]);
  }
  return out;
}

function helpText(args) {
  try {
    return execFileSync(process.execPath, ['bin/bukio.js', ...args, '--help'], {
      cwd: root,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    });
  } catch {
    return null;
  }
}

/** Walk the whole command tree; returns {"": rootHelp, "bank": ..., ...}. */
export function captureHelp() {
  const out = {};
  const queue = [];
  const seen = new Set();
  for (const name of childrenOf(helpText([]) ?? '')) queue.push(name);
  out[''] = helpText([]);
  while (queue.length) {
    const p = queue.shift();
    if (seen.has(p)) continue;
    seen.add(p);
    if (p.split(' ').length > MAX_DEPTH) continue;
    const text = helpText(p.split(' '));
    if (text == null) continue;
    out[p] = text;
    for (const c of childrenOf(text)) queue.push(`${p} ${c}`);
  }
  // This port renders PDFs itself (src/pdf.rs, no browser involved), so the JS
  // help's "headless Chromium" would be a lie in the binary's own --help.
  // Adapt it here, in the generator the drift guard also calls, so both sides
  // see the same text.
  for (const [k, v] of Object.entries(out)) {
    if (typeof v === 'string') out[k] = v.replaceAll('headless Chromium', 'native renderer');
  }
  return out;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const tree = captureHelp();
  const file = path.join(root, 'src', 'help.json');
  writeFileSync(file, `${JSON.stringify(tree, null, 1)}\n`);
  const deepest = Object.keys(tree).sort((a, b) => b.split(' ').length - a.split(' ').length)[0];
  console.log(`${Object.keys(tree).length} command paths -> src/help.json (deepest: '${deepest}')`);
}
