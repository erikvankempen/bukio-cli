/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// CLI wiring — commander program with global --json/--db/--actor flags.
import { Command } from 'commander';
import os from 'node:os';
import path from 'node:path';
import { make as initCmd } from './init.js';
import { make as entryCmd } from './entry.js';
import { make as accountCmd } from './account.js';
import { make as reportCmd } from './report.js';
import { make as auditCmd } from './audit.js';
import { make as backupCmd } from './backup.js';
import { make as bankCmd } from './bank.js';
import { make as vatCmd } from './vat.js';
import { make as recurringCmd } from './recurring.js';
import { make as invoiceCmd } from './invoice.js';
import { make as yearEndCmd } from './year-end.js';
import { make as fxCmd } from './fx.js';
import { make as mcpCmd } from './mcp.js';
import { make as complianceCmd } from './compliance.js';
import { make as importCmd } from './import.js';
import { make as exportCmd } from './export.js';
import { make as monthEndCmd } from './month-end.js';
import { make as companyCmd } from './company.js';
import { make as assetsCmd } from './assets.js';
import { make as paymentsCmd } from './payments.js';
import { make as itemCmd } from './item.js';
import { make as attachCmd } from './attach.js';
import { make as updateCmd } from './update.js';
import { make as actorCmd } from './actor.js';
import { make as serverCmd } from './server.js';
import { remotePreAction } from './remote.js';
import { actorError } from '../core/actor.js';
import { signCommand } from './util.js';
import { setPendingSignature } from '../audit/index.js';

export async function runCli(argv) {
  const program = new Command();
  program
    .name('bukio')
    .description('Agent-first bookkeeping for Dutch SMEs — SQLite, VAT-optional')
    .version('0.15.1')
    .option('--json', 'machine-readable JSON output')
    .option('--db <path>', 'database file', process.env.BUKIO_DB || path.join(os.homedir(), '.bukio', 'bukio.db'))
    .option('--actor <who>', "acting entity '<role>:<name>' — e.g. agent:bartholomeus, human:erik (or BUKIO_ACTOR env; required)", undefined)
    .option('--sign-key <path>', 'explicit private-key file to sign with (default: the actor\'s key in <config>/keys)', undefined)
    .option('--server <url>', 'remote bukio server — POST a signed command envelope instead of running locally (or BUKIO_SERVER env)', undefined)
    .option('--locale <code>', 'output language for human-facing text (or BUKIO_LOCALE env; default: the company locale, else English)', undefined)
    .showHelpAfterError();

  initCmd(program);
  entryCmd(program);
  accountCmd(program);
  reportCmd(program);
  auditCmd(program);
  backupCmd(program);
  bankCmd(program);
  vatCmd(program);
  recurringCmd(program);
  invoiceCmd(program);
  yearEndCmd(program);
  fxCmd(program);
  mcpCmd(program);
  complianceCmd(program);
  importCmd(program);
  exportCmd(program);
  monthEndCmd(program);
  companyCmd(program);
  assetsCmd(program);
  paymentsCmd(program);
  itemCmd(program);
  attachCmd(program);
  updateCmd(program);
  actorCmd(program);
  serverCmd(program);

  // Named-actor enforcement: every action must identify as '<role>:<name>'
  // (agent:bartholomeus, human:erik) so the audit trail always names who acted.
  // Then the Tier 0 sign gate: build the canonical digest, auto-sign, verify
  // against the per-company registry, and refuse before dispatch when the
  // company enforces signing. The signature bundle lands on the command's
  // audit row(s) via setPendingSignature.
  //
  // Three preAction modes:
  //   child-exec  (BUKIO_REMOTE_EXEC=1) — spawned by a `server start` daemon
  //               that ALREADY verified the envelope; attach the verified
  //               signature bundle and run locally (no re-signing, no
  //               re-verification).
  //   remote      (--server <url> / BUKIO_SERVER) — this device signs the
  //               envelope and the server runs it; output is byte-identical.
  //   local       (default) — the classic sign-and-run path.
  program.hook('preAction', async (_thisCmd, actionCmd) => {
    if (process.env.BUKIO_REMOTE_EXEC === '1') {
      try {
        setPendingSignature(JSON.parse(process.env.BUKIO_REMOTE_SIG ?? 'null'));
      } catch {
        setPendingSignature(null);
      }
      return;
    }
    const o = actionCmd.optsWithGlobals();
    const issue = actorError(o.actor ?? process.env.BUKIO_ACTOR ?? null);
    if (issue) {
      if (o.json) {
        console.log(JSON.stringify({ ok: false, error: issue }));
        process.exit(1);
      }
      program.error(`error [${issue.code}]: ${issue.message}`, { exitCode: 1 });
    }
    const ctx = {
      json: Boolean(o.json),
      dbPath: o.db,
      actor: o.actor ?? process.env.BUKIO_ACTOR,
      dryRun: Boolean(o.dryRun),
      server: o.server ?? process.env.BUKIO_SERVER ?? null,
    };
    if (ctx.server) {
      try {
        const code = await remotePreAction(ctx, actionCmd, argv.slice(2), ctx.server);
        if (code !== undefined) process.exit(code);
      } catch (err) {
        const error = { code: err.code || 'ERROR', message: err.message };
        if (o.json) {
          console.log(JSON.stringify({ ok: false, error }));
        } else {
          console.error(`error [${error.code}]: ${error.message}`);
        }
        process.exit(1);
      }
      return;
    }
    try {
      setPendingSignature(signCommand(ctx, actionCmd));
    } catch (err) {
      const error = { code: err.code || 'ERROR', message: err.message };
      if (o.json) {
        console.log(JSON.stringify({ ok: false, error }));
        process.exit(1);
      }
      program.error(`error [${error.code}]: ${error.message}`, { exitCode: 1 });
    }
  });

  await program.parseAsync(argv);
}
