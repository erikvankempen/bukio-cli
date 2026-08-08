/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio update — self-update from the GitHub main branch.
// Plans first (--dry-run: what would come in, what would be overwritten),
// then resets to origin/main on explicit --yes. The company database (if
// one exists) gets an audit row; the books themselves are never touched.
import path from 'node:path';
import { ensureDb, makeCtx, output, fail } from './util.js';
import { planUpdate, runUpdate, resolveRepoRoot } from '../update/index.js';

export function make(program) {
  program
    .command('update')
    .description('fetch the latest commits from the GitHub main branch and reset to them (warns — overwrites local customizations)')
    .option('--yes', 'confirm the overwrite: reset the working tree to origin/main, losing local modifications and local commits')
    .option('--repo <path>', 'repository root to update (default: the installation this binary runs from)')
    .option('--trust-remote', 'skip the official-repository check (for forks/mirrors of bukio-cli)')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const repoPath = opts.repo ? path.resolve(opts.repo) : resolveRepoRoot();
        // the company DB is only used for the audit row; update works without one
        const db = ensureDb(ctx, { mustExist: false });
        try {
          if (ctx.dryRun) {
            const plan = planUpdate({ repoPath, verifyRemote: !opts.trustRemote });
            output(ctx, { ...plan, dryRun: true, action: 'update' }, (d) => {
              if (d.up_to_date) {
                console.log(`already up to date on ${d.branch} (${d.current_version ?? '?'})`);
                return;
              }
              console.log(`plan: update bukio-cli ${d.current_version ?? '?'} -> origin/${d.branch} (${d.incoming_count} commit${d.incoming_count === 1 ? '' : 's'})`);
              for (const c of d.incoming.slice(0, 20)) console.log(`  ${c}`);
              if (d.incoming.length > 20) console.log(`  … and ${d.incoming.length - 20} more`);
              if (d.warning) {
                console.log(`⚠️  ${d.warning}`);
                for (const f of d.modified_files) console.log(`  modified: ${f}`);
                for (const c of d.local_commits) console.log(`  local commit: ${c}`);
              }
              console.log('(dry run — nothing written; rerun without --dry-run and add --yes to proceed)');
            });
            return;
          }
          const result = runUpdate({ repoPath, yes: Boolean(opts.yes), db, actor: ctx.actor, verifyRemote: !opts.trustRemote });
          output(ctx, result, (d) => {
            if (!d.updated) {
              console.log(`already up to date on ${d.branch} (${d.current_version ?? '?'})`);
              return;
            }
            console.log(`updated bukio-cli ${d.from_sha.slice(0, 7)} -> ${d.to_sha.slice(0, 7)} (${d.commits_applied} commits, version ${d.version_after ?? '?'})`);
            if (d.deps_installed) console.log('  dependencies reinstalled (package.json changed)');
            if (d.deps_error) console.log(`  ⚠️  npm install failed: ${d.deps_error} — run it manually`);
          });
        } finally {
          if (db) db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
