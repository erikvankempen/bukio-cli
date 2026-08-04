// bukio audit — append-only audit log reader.
import { list } from '../audit/index.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

export function make(program) {
  program
    .command('audit')
    .description('read the append-only audit log')
    .option('--since <iso-ts>', 'only entries at or after this timestamp')
    .option('--by <who>', 'only entries by this actor')
    .option('--limit <n>', 'max rows', '50')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const rows = list(db, {
            since: opts.since || null,
            actor: opts.by || null,
            limit: parseInt(opts.limit, 10) || 50,
          });
          const data = { entries: rows };
          output(ctx, data, (d) => {
            table(d.entries, [
              { key: 'id', label: '#' },
              { key: 'ts', label: 'timestamp' },
              { key: 'actor', label: 'actor' },
              { key: 'action', label: 'action' },
              { key: 'command', label: 'command' },
              { key: 'outcome', label: 'outcome' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
