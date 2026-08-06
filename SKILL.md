---
name: bukio-cli
description: Drive bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs. Book invoices, archive the originals, keep the books balanced.
---

# bukio-cli — agent workflow

Full manual: `AGENTS.md` (house rules, JSON contracts, account codes, error codes,
anti-patterns). This skill is the **agent-facing workflow** for the two things
you'll do most: booking documents and keeping the books verifiable.

## When to use

- User sends an invoice PDF/photo/scan: "book this", "hoe boek ik dit?"
- Register an inkoopfactuur (purchase) or verkoopfactuur (sale)
- Month-end: bank import → match → VAT readout, or the `month-end` close check
- Any mutation of the ledger (entry, reversal, VAT, assets, payables)

## Environment

| What | How |
|------|-----|
| DB | `--db <path>` or `BUKIO_DB` (default `~/.bukio/bukio.db`) |
| Actor | `--actor '<role>:<name>'` or `BUKIO_ACTOR` — required (e.g. `agent:bartholomeus`) |
| JSON | `--json` on every command; exit 0 = success, 1 = failure |
| Verify | after any mutation: `bukio report trial-balance --json` → `data.balanced === true` |

House rules (non-negotiable): dry-run every mutation first · never touch the
SQLite file directly · never delete posted entries (reverse instead) · money is
integer cents · **archive every source document (see below)**.

## Booking an invoice (the core loop)

1. **Extract** — read the invoice text (OCR for scans/images). Recompute the
   VAT: `vat = round(net × rate)` per rate, `gross = net + vat`. If the stated
   VAT differs beyond rounding, STOP and ask.
2. **Map** — account from the chart (defaults: 4000 goods, 4340 software/SaaS,
   4300 kantoor/general, 4310 accountant, 4320 insurance, 1800 equipment
   >€450, 4500 bank charges; sales → 8000/8100), VAT code (21/9/0/V/R/RE/marge),
   counter leg (1100 bank if paid, 2000 crediteuren if unpaid, 1200 debiteuren
   for sales).
3. **Book** — VAT module on → `bukio vat book --postings "4300:100.00@21,1100:-121.00" --post`
   (never hand-build VAT legs with `entry add`). VAT off/KOR → gross to the
   expense account, no VAT legs. Always `--dry-run` first, always `--actor`.
4. **Archive the original (mandatory)** — before finishing:

   ```bash
   mkdir -p ~/.bukio/invoices
   cp <original>.pdf ~/.bukio/invoices/<YYYY-MM-DD>_<vendor-slug>_<invoice-number>.pdf
   ```

   - Location: `~/.bukio/invoices/` next to the live DB — **outside the git
     repo; never commit source documents** (`.gitignore` blocks `*.pdf` anyway).
   - Name: `date_vendor-invoicenumber.ext` (e.g. `2026-07-10_acme-bv_F2026-123.pdf`).
     Keep whatever format arrived (.pdf, .jpg, .png, .xml, .eml) — don't convert.
   - Other DBs archive next to their own file (e.g. `demo.db` → `demo-invoices/`).
   - Entry description carries the same reference (`"Vendor - F2026-123"`).
5. **Verify** — trial balance balanced; `vat readout --period` reflects the
   purchase; `audit` shows your actor; archived file exists (`ls`).

## Month-end loop (the standard close)

```bash
bukio bank import --file stmt.camt.053 --iban <IBAN> --dry-run   # then for real
bukio bank match auto --json                                     # reconcile
bukio recurring preview --as-of YYYY-MM-DD --json                # what's due
bukio recurring run --as-of YYYY-MM-DD --dry-run --json          # then for real
bukio month-end --period YYYY-MM --json                          # close check
bukio vat readout --period YYYY-Qn --json                        # OB fields 1a-5d
```

`month-end` is read-only: drafts, unmatched bank, draft/overdue invoices, due
recurring, period totals + profit, `warnings[]`. Nothing is closed automatically.

## Pitfalls

- **OCR lies** — verify every number against the maths; ask when in doubt.
- **Verlegd must book the 1500 claim leg** (`1500:-<vat>`), else OB 5d overstates.
- **Marge cannot be auto-split** — book manually, no VAT split.
- **KOR companies book the gross** — no deduction, no VAT module.
- **Never re-export a SEPA batch** — the stored MsgId guard exists because
  re-uploading pays twice.
- **Archive before finishing** — a crash mid-booking must not lose the paper.

## Related

- `AGENTS.md` in this repo — full manual (JSON contracts, error codes §7,
  anti-patterns §8, capability boundaries §9)
- `bukio-invoice-booking` skill (Hermes) — extraction script + decision tables
