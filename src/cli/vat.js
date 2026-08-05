// bukio vat — optional VAT module: enable, book, OB manual-filing readout.
import { formatAmount } from '../core/money.js';
import {
  bookVatEntry, enableVatModule, expandVatPostings, isVatEnabled, listVatCodes,
  markFiled, obReadout, parseVatPostingSpecs,
} from '../vat/index.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

import { getFxRate, parseRate, toEurPostings, resolveRate } from '../fx/index.js';

/** Convert posting specs to EUR when --currency given; auto rate lookup + ECB fallback. */
async function applyFxToSpecs(db, specs, { currency, rate, date, actor }) {
  const rateX10000 = await resolveRate(db, { currency, rate, date, actor });
  return toEurPostings(specs, { currency, rateX10000 });
}

function fmtEntry(entry) {
  return {
    id: entry.id, date: entry.date, description: entry.description, state: entry.state,
    postings: entry.postings.map((p) => ({
      account_code: p.account_code, account_name: p.account_name,
      amount_cents: p.amount_cents, amount: formatAmount(p.amount_cents),
      vat_code: p.vat_code_id ? undefined : null,
      vat_amount_cents: p.vat_amount_cents,
      vat_amount: p.vat_amount_cents == null ? null : formatAmount(p.vat_amount_cents),
    })),
  };
}

export function make(program) {
  const vat = program.command('vat').description('VAT module (optional)');

  vat
    .command('enable')
    .description('enable the VAT module: VAT accounts + VAT codes (idempotent)')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          if (ctx.dryRun) {
            output(ctx, {
              action: 'enable VAT module',
              accounts: ['1500', '2500'],
              codes: ['21', '9', '0', 'V', 'R', 'RE', 'M', 'P'],
              dryRun: true,
            }, (d) => {
              console.log('plan: enable VAT module');
              console.log(`  accounts: ${d.accounts.join(', ')}`);
              console.log(`  codes:    ${d.codes.join(', ')}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          const result = enableVatModule(db, { actor: ctx.actor });
          output(ctx, { ...result, dryRun: false }, (d) => {
            console.log(`VAT module enabled: ${d.accounts.length} VAT accounts, ${d.codes.length} VAT codes`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  vat
    .command('codes')
    .description('list VAT codes')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const codes = listVatCodes(db);
          const data = { codes: codes.map((c) => ({
            code: c.code, rate_bp: c.rate_bp, rate: (c.rate_bp / 100).toFixed(1) + '%',
            type: c.type, eu_reverse: Boolean(c.eu_reverse), description: c.description,
          })) };
          output(ctx, data, (d) => {
            table(d.codes, [
              { key: 'code', label: 'code' },
              { key: 'rate', label: 'rate' },
              { key: 'type', label: 'type' },
              { key: 'eu_reverse', label: 'eu' },
              { key: 'description', label: 'description' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  vat
    .command('book')
    .description('book a VAT-aware entry — tag a net posting with @VATCODE (e.g. 8000:-100.00@21)')
    .requiredOption('--date <yyyy-mm-dd>', 'entry date')
    .requiredOption('--desc <description>', 'description')
    .requiredOption('--postings <CODE:AMOUNT[@VATCODE]>', 'posting specs, repeatable or comma-separated')
    .option('--source <source>', 'manual|bank|invoice|agent', 'manual')
    .option('--source-ref <ref>', 'source reference')
    .option('--currency <ISO>', 'postings are in this foreign currency; converted to EUR (needs a rate)')
    .option('--rate <n>', 'FX rate (1 EUR = n units); auto-looked-up on/before the date when omitted')
    .option('--post', 'post the entry immediately')
    .option('--dry-run', 'show the expanded plan without writing')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const specs = parseVatPostingSpecs(opts.postings);
          const converted = opts.currency
            ? await applyFxToSpecs(db, specs, { ...opts, actor: ctx.actor })
            : specs;
          if (ctx.dryRun) {
            const expanded = expandVatPostings(db, converted);
            output(ctx, {
              action: 'book VAT entry (expanded)',
              date: opts.date, description: opts.desc,
              currency: opts.currency ?? null,
              postings: expanded.map((p) => ({
                code: p.code, amount_cents: p.amountCents, amount: formatAmount(p.amountCents),
                vat_code: p.vatCode, vat_amount: p.vatAmountCents == null ? null : formatAmount(p.vatAmountCents),
                fx_currency: p.fxCurrency, fx_amount_cents: p.fxAmountCents,
              })),
              post: Boolean(opts.post), dryRun: true,
            }, (d) => {
              console.log(`plan: VAT entry ${d.date} "${d.description}"${d.currency ? ` (${d.currency} -> EUR)` : ''}`);
              for (const p of d.postings) {
                const fx = p.fx_currency ? `  [${p.fx_currency} ${p.fx_amount_cents != null ? (p.fx_amount_cents / 100).toFixed(2) : ''}]` : '';
                console.log(`  ${p.code}  ${p.amount.padStart(12)}${fx}${p.vat_code ? `  @${p.vat_code} (${p.vat_amount})` : ''}`);
              }
              console.log('(dry run — nothing written)');
            });
            return;
          }
          const { entry, expanded } = bookVatEntry(db, {
            date: opts.date, description: opts.desc, postings: converted,
            source: opts.source, sourceRef: opts.sourceRef ?? null,
            actor: ctx.actor, post: Boolean(opts.post),
          });
          output(ctx, { entry: fmtEntry(entry), expanded: expanded.map((p) => ({ code: p.code, vat_code: p.vatCode })) },
            (e) => {
              console.log(`entry #${e.entry.id}  [${e.entry.state}]  ${e.entry.date}  ${e.entry.description}`);
              for (const p of e.entry.postings) {
                const fx = p.fx_currency ? `  [${p.fx_currency} ${p.fx_amount_cents != null ? (p.fx_amount_cents / 100).toFixed(2) : ''}]` : '';
                console.log(`  ${p.account_code}  ${p.account_name.padEnd(28)} ${p.amount.padStart(12)}${fx}${p.vat_amount ? `  vat ${p.vat_amount}` : ''}`);
              }
            });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  vat
    .command('readout')
    .description('OB-aangifte manual-filing readout (fields 1a-5d) for a period')
    .requiredOption('--period <period>', 'YYYY-Qn (quarter) or YYYY-MM (month)')
    .option('--mark-filed', 'record that this period was filed manually')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          if (opts.markFiled) {
            const result = markFiled(db, { period: opts.period, actor: ctx.actor });
            output(ctx, { ...result, dryRun: false }, (d) => {
              console.log(`marked ${d.period} as filed`);
            });
            return;
          }
          const readout = obReadout(db, { period: opts.period });
          const fields = Object.fromEntries(
            Object.entries(readout.fields).map(([k, v]) => [k, { cents: v, amount: formatAmount(v) }]),
          );
          const data = {
            period: readout.period, from: readout.from, to: readout.to,
            fields,
            to_pay_cents: readout.to_pay_cents,
            to_pay: readout.to_pay,
            note: readout.note,
          };
          output(ctx, data, (d) => {
            console.log(`OB-AANGIFTE ${d.period} (${d.from} .. ${d.to}) — manual filing aid`);
            console.log('  1a  omzet hoog           ', d.fields['1a'].amount);
            console.log('  1b  omzet laag           ', d.fields['1b'].amount);
            console.log('  1c  omzet 0/vrijgesteld  ', d.fields['1c'].amount);
            console.log('  1d  privégebruik         ', d.fields['1d'].amount);
            console.log('  3a  inkopen hoog         ', d.fields['3a'].amount);
            console.log('  3b  inkopen laag         ', d.fields['3b'].amount);
            console.log('  3c  inkopen 0/verlegd    ', d.fields['3c'].amount);
            console.log('  4a  verlegd binnenland   ', d.fields['4a'].amount);
            console.log('  4b  verlegd EU           ', d.fields['4b'].amount);
            console.log('  5a  verschuldigde btw    ', d.fields['5a'].amount);
            console.log('  5b  voorbelasting        ', d.fields['5b'].amount);
            console.log('  5d  te betalen/ontvangen ', d.to_pay);
            console.log(`  -> enter these amounts in Mijn Belastingdienst Zakelijk`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
