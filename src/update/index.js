/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio update — self-update from the GitHub main branch.
// `git fetch` the official remote, then `git reset --hard origin/main`.
// RESET --HARD OVERWRITES LOCAL CUSTOMIZATIONS: tracked modifications and
// local commits are lost (untracked files are kept). The flow therefore
// always plans first (--dry-run shows exactly what would be overwritten)
// and the real run refuses without explicit --yes confirmation.
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { record } from '../audit/index.js';

export const OFFICIAL_REMOTE = /github\.com[:\/]erikvankempen\/bukio-cli(\.git)?$/;

function updateError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

function git(repoPath, args) {
  try {
    // trailing-only trim: porcelain output KEEPS its leading space column
    // (' M file' = worktree-modified); a full trim() would eat it
    return execFileSync('git', args, { cwd: repoPath, encoding: 'utf8' }).replace(/\s+$/, '');
  } catch (err) {
    const stderr = (err.stderr || err.message || '').toString().trim();
    throw updateError('UPDATE_GIT_FAILED', `git ${args[0]} failed: ${stderr || err.message}`);
  }
}

/** The repo root this module ships from (src/update/ -> repo root). */
export function resolveRepoRoot() {
  return path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
}

function readPackageVersion(repoPath) {
  try {
    return JSON.parse(readFileSync(path.join(repoPath, 'package.json'), 'utf8')).version ?? null;
  } catch {
    return null;
  }
}

/**
 * Fetch + inspect WITHOUT mutating anything. Returns the full plan: what
 * would come in, what would be lost. Throws UPDATE_NOT_A_CLONE (no .git),
 * UPDATE_WRONG_REMOTE (not the official repo), UPDATE_NO_REMOTE_BRANCH.
 */
export function planUpdate({ repoPath, remote = 'origin', branch = 'main', verifyRemote = true }) {
  if (!existsSync(path.join(repoPath, '.git'))) {
    throw updateError(
      'UPDATE_NOT_A_CLONE',
      `'${repoPath}' is not a git clone — \`bukio update\` works on a cloned installation; an npm -g install must be updated with \`npm update -g bukio-cli\``,
    );
  }
  const remoteUrl = git(repoPath, ['config', '--get', `remote.${remote}.url`]);
  if (verifyRemote && !OFFICIAL_REMOTE.test(remoteUrl)) {
    throw updateError('UPDATE_WRONG_REMOTE', `remote '${remote}' (${remoteUrl}) is not the official github.com/erikvankempen/bukio-cli repository`);
  }
  git(repoPath, ['fetch', remote, branch]);
  const currentSha = git(repoPath, ['rev-parse', 'HEAD']);
  let targetSha;
  try {
    targetSha = git(repoPath, ['rev-parse', `${remote}/${branch}`]);
  } catch {
    throw updateError('UPDATE_NO_REMOTE_BRANCH', `no ${remote}/${branch} ref after fetching ${remote}`);
  }
  const incoming = git(repoPath, ['log', '--oneline', `${currentSha}..${targetSha}`]).split('\n').filter(Boolean);
  const localCommits = git(repoPath, ['log', '--oneline', `${targetSha}..${currentSha}`]).split('\n').filter(Boolean);
  // porcelain: "XY path"; tracked modifications = anything that is NOT "??"
  const status = git(repoPath, ['status', '--porcelain']).split('\n').filter(Boolean);
  const modifiedFiles = status.filter((l) => !l.startsWith('??')).map((l) => l.slice(3));
  const untrackedCount = status.filter((l) => l.startsWith('??')).length;
  const packageJsonChanged = git(repoPath, ['diff', '--name-only', `${currentSha}..${targetSha}`, '--', 'package.json']).length > 0;

  const warning = modifiedFiles.length || localCommits.length
    ? `OVERWRITES LOCAL CUSTOMIZATIONS: ${modifiedFiles.length} modified file(s) and ${localCommits.length} local commit(s) will be lost by resetting to ${remote}/${branch}. Untracked files are kept.`
    : null;

  return {
    repo_path: repoPath, remote, branch, remote_url: remoteUrl,
    current_sha: currentSha, target_sha: targetSha,
    current_version: readPackageVersion(repoPath),
    incoming_count: incoming.length, incoming,
    local_commits: localCommits, modified_files: modifiedFiles,
    untracked_count: untrackedCount,
    package_json_changed: packageJsonChanged,
    up_to_date: currentSha === targetSha,
    warning,
  };
}

/**
 * Fetch + reset --hard origin/<branch>. Requires `yes` when the plan shows
 * anything to overwrite (or always when not up to date — the plan already
 * checked). Records an audit row when a company db is provided.
 */
export function runUpdate({ repoPath, remote = 'origin', branch = 'main', yes = false, db = null, actor = null, verifyRemote = true }) {
  const plan = planUpdate({ repoPath, remote, branch, verifyRemote });
  if (plan.up_to_date) {
    return { action: 'update', updated: false, ...plan };
  }
  if (!yes) {
    throw updateError(
      'UPDATE_CONFIRM_REQUIRED',
      `refusing to reset to ${remote}/${branch} without confirmation — pass --yes (this would overwrite local customizations; run --dry-run to see the plan)`,
    );
  }
  const fromSha = plan.current_sha;
  git(repoPath, ['reset', '--hard', `${remote}/${branch}`]);
  let depsInstalled = false;
  let depsError = null;
  if (plan.package_json_changed) {
    try {
      execFileSync('npm', ['install'], { cwd: repoPath, encoding: 'utf8', stdio: 'ignore' });
      depsInstalled = true;
    } catch (err) {
      depsError = (err.stderr || err.message || '').toString().trim().split('\n').slice(-3).join(' ');
    }
  }
  const toSha = git(repoPath, ['rev-parse', 'HEAD']);
  const versionAfter = readPackageVersion(repoPath);
  const result = {
    action: 'update', updated: true, from_sha: fromSha, to_sha: toSha,
    commits_applied: plan.incoming_count, version_after: versionAfter,
    deps_installed: depsInstalled, deps_error: depsError,
  };
  if (db) {
    record(db, {
      actor: actor ?? 'human', action: 'update', command: 'update',
      args: { from_sha: fromSha, to_sha: toSha, commits: plan.incoming_count, version_after: versionAfter, deps_installed: depsInstalled },
      outcome: depsError ? 'warn' : 'ok',
    });
  }
  return result;
}
