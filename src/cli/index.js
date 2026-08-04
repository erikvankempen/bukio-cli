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

export async function runCli(argv) {
  const program = new Command();
  program
    .name('bukio')
    .description('Agent-first bookkeeping for Dutch SMEs — SQLite, VAT-optional')
    .version('0.4.0')
    .option('--json', 'machine-readable JSON output')
    .option('--db <path>', 'database file', process.env.BUKIO_DB || path.join(os.homedir(), '.bukio', 'bukio.db'))
    .option('--actor <who>', 'acting entity (human or agent:<name>)', process.env.BUKIO_ACTOR || 'human')
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

  await program.parseAsync(argv);
}
