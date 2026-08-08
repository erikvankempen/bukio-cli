# bukio-cli — test report

**Latest run:** 2026-08-08 22:18:54 UTC — **✅ 568 passing · 0 failing (568 tests)**
**Command:** `npm test` (per-file `node --test --test-reporter=tap`)

## All tests

### accounts.test.js — chart of accounts CRUD + CSV chart import

7 passing · 0 failing

    - ✅ createAccount: valid account lands in the chart
    - ✅ createAccount: rejects duplicates and invalid input
    - ✅ deactivate/reactivate lifecycle blocks new postings
    - ✅ importChartCsv: imports valid rows, skips duplicates and invalid rows
    - ✅ importChartCsv: header validation
    - ✅ importChartCsv: quoted values with commas parse
    - ✅ listAccounts: type filter and includeInactive

### actor.test.js — named-actor enforcement (`<role>:<name>` required)

7 passing · 0 failing

    - ✅ isValidActor: role:name formats
    - ✅ actorError: helpful messages for missing and malformed actors
    - ✅ CLI: missing actor fails with ACTOR_REQUIRED
    - ✅ CLI: bare role without a name is rejected (INVALID_ACTOR)
    - ✅ CLI: named actor works; JSON error shape on --json
    - ✅ CLI: BUKIO_ACTOR env satisfies the requirement
    - ✅ CLI: BUKIO_ACTOR env is recorded in the audit trail

### agent-layer.test.js — MCP server, FX/ECB, tool gates, compliance calendar

21 passing · 0 failing

    - ✅ fx: parseRate and convertFx — integer math, round-half-up
    - ✅ fx: setFxRate upsert + audit; getFxRate exact then latest-on/before
    - ✅ fx: toEurPostings attaches the original amounts
    - ✅ fx: entry add with currency books EUR + keeps the original amounts (reversal too)
    - ✅ fx: vat book with currency — VAT legs computed on the EUR amounts
    - ✅ fx: invalid currency on a posting is rejected
    - ✅ ecb: parses SDMX observations and falls back to the last business day
    - ✅ ecb: 404 (unknown currency) -> null; network failure -> ECB_FETCH_FAILED
    - ✅ fx: missing rate auto-fetches from ECB, stores it, and reuses it
    - ✅ fx: BUKIO_FX_NO_FETCH blocks the ECB fallback
    - ✅ fx: ECB has no rate for the currency -> ECB_RATE_NOT_AVAILABLE
    - ✅ compliance: quarterly deadlines
    - ✅ compliance: jaarrekening deadline is 13 months after the fiscal year end
    - ✅ compliance: calendar shows obligations, statuses flip with filings
    - ✅ compliance: closed books show on the jaarrekening obligation
    - ✅ MCP: initialize + tools/list + read-only calls work end-to-end
    - ✅ MCP: non-object JSON-RPC messages get Invalid Request, server survives
    - ✅ MCP: mutations are plan-only by default; execute books with the actor
    - ✅ MCP: assets_run books DEPRECIATION, not recurring entries (import-collision regression)
    - ✅ MCP: contact_add preserves postal_code and vat_id (regression)
    - ✅ MCP: BUKIO_MCP_READONLY blocks execution

### assets.test.js — fixed assets: schemes, mid-life adoption, runs, disposal, activastaat

25 passing · 0 failing

    - ✅ ensureDefaultScheme: creates the standard 5y linear scheme lazily
    - ✅ createScheme: rejects duplicate names and bad methods
    - ✅ scheduleDepreciation: linear 60m is cents-exact and remainder-adjusted
    - ✅ scheduleDepreciation: degressief double-declining with switch to linear
    - ✅ scheduleDepreciation: stops at the residual, never overshoots
    - ✅ addAsset: standard 5y linear, first run on the 1st of the month
    - ✅ addAsset: mid-life adoption keeps only the remaining depreciation
    - ✅ addAsset: cum-dep at recognition above cost minus residual is rejected
    - ✅ addAsset: recognises an already fully depreciated asset as fully_depreciated
    - ✅ addAsset: account type validation
    - ✅ addAsset: missing entry link fails ENTRY_NOT_FOUND
    - ✅ addAsset: dry-run writes nothing
    - ✅ runDue: books monthly depreciation, idempotent per asset-month
    - ✅ runDue: paused assets do not run; resume restarts them
    - ✅ runDue: dry-run plans but books nothing
    - ✅ runDue: auto-completes to fully_depreciated at the residual
    - ✅ runDue: books on cum-dep account when provided, else on the asset account
    - ✅ disposeAsset: sale with winst books the full entry and closes the asset
    - ✅ disposeAsset: scrap (proceeds 0) books a verlies
    - ✅ disposeAsset: rejects double disposal and bad dates
    - ✅ disposeAsset: dry-run books nothing
    - ✅ register: book values and totals as of a date
    - ✅ register: disposal dates and proceeds surface in the register
    - ✅ trial balance stays balanced through the whole lifecycle
    - ✅ disposeAsset: entry + asset status are atomic (no orphaned entry on rollback)

### attachments.test.js — in-DB/file document attachments: add/list/show/remove, 25 MB cap, dedupe, metadata-only lists, audit

15 passing · 0 failing

    - ✅ attach add (db mode): stores BLOB, round-trips byte-identical, infers mime
    - ✅ attach add: works for entries too
    - ✅ attach add: validation errors
    - ✅ attach list: metadata only, no data column payload
    - ✅ attach remove: deletes row + audits; unknown id errors
    - ✅ attach add: dry-run writes nothing and audits nothing
    - ✅ attach add: file mode copies to <db>-attachments/<sha256> and remove deletes it
    - ✅ attach get: file-mode with missing file on disk → ATTACHMENT_FILE_MISSING
    - ✅ cli: attach add/list/show --out/remove round-trip with audit
    - ✅ cli: attach add rejects both refs, and unknown store
    - ✅ cli: attach dry-run writes nothing
    - ✅ cli: attach file mode end-to-end
    - ✅ migration 013 applies on fresh init (attachments table exists)
    - ✅ attachmentsDir convention: demo.db → demo-attachments/
    - ✅ file-mode attachments dir is created under the DB dir (regression)

### audit.test.js — append-only audit log invariants

3 passing · 0 failing

    - ✅ record + list with filters
    - ✅ audit log is append-only: UPDATE and DELETE are blocked
    - ✅ args null is stored and read back as null

### backup.test.js — encrypted backups (AES-256-GCM), keep-N rotation, tamper detection, audited restore

9 passing · 0 failing

    - ✅ backup --encrypt: magic header, round-trips byte-identical via restore
    - ✅ restore: encrypted file without passphrase → BACKUP_PASSPHRASE_REQUIRED; wrong → BACKUP_PASSPHRASE_WRONG
    - ✅ restore: passphrase from BUKIO_BACKUP_PASSPHRASE env works
    - ✅ tampered encrypted backup → BACKUP_PASSPHRASE_WRONG
    - ✅ unit: encrypt/decrypt round-trip and wrong key
    - ✅ --keep N prunes oldest backups in the default folder; dry-run deletes nothing
    - ✅ --keep validation: non-integer, zero, and with --out all rejected
    - ✅ plain backup/restore still works (regression) + both actions audited
    - ✅ pruneBackups: empty/missing folder is a no-op

### bank.test.js — CAMT.053/CSV import, idempotency, matching/reconciliation

18 passing · 0 failing

    - ✅ parseCamt053: CRDT positive, DBIT negative, counterparty + description
    - ✅ parseCamt053: rejects non-CAMT input
    - ✅ parseBankAmount: Dutch and international formats
    - ✅ parseBankCsv: Rabo-style export with Af/Bij sign
    - ✅ parseBankCsv: missing required columns rejected
    - ✅ importTransactions: idempotent via hash (duplicates skipped)
    - ✅ previewImport: dry-run counts without writing
    - ✅ getOrCreateBankAccount: validates IBAN and links to ledger account
    - ✅ listBankAccounts: balance and counts
    - ✅ postFromTransaction: posts bank + counter leg and reconciles
    - ✅ postFromTransaction: refuses already-matched transactions
    - ✅ linkTransaction: links a posted entry and guards
    - ✅ autoMatch: exact and fuzzy matching, dry-run writes nothing
    - ✅ autoMatch: two same-amount transactions never claim the same entry in one run
    - ✅ autoMatch: two same-amount transactions match TWO distinct entries (param order regression)
    - ✅ autoMatch: outside the window stays unmatched
    - ✅ setTransactionState: ignore and re-open
    - ✅ suggestUnmatched: proposes expense/income accounts

### cli.test.js — CLI end-to-end: init, entries, reports, backup/restore

26 passing · 0 failing

    - ✅ init --dry-run: shows plan, creates nothing
    - ✅ init: creates company + 30-account chart with VAT on
    - ✅ init: second init fails with ALREADY_INITIALISED
    - ✅ entry add --dry-run: plans without writing
    - ✅ entry add: rejects malformed posting spec and unknown account
    - ✅ entry add --post + trial balance + audit end-to-end
    - ✅ entry reverse: contra-entry keeps the trial balance balanced
    - ✅ report trial-balance csv: TOTAAL row net is 0.00 for a balanced ledger (regression)
    - ✅ commands fail cleanly when no database exists
    - ✅ --actor is recorded on entries and audit
    - ✅ account add/list/show/deactivate flow
    - ✅ account import: dry-run validates, real import creates
    - ✅ report balans/pnl/journal: JSON + CSV + XLSX export
    - ✅ report balans --as-of is respected
    - ✅ backup + restore roundtrip
    - ✅ bank import (CAMT + CSV), idempotency, match --post, ignore
    - ✅ bank match --auto links posted entries (exact)
    - ✅ vat enable/book/readout/mark-filed end-to-end
    - ✅ vat: module off blocks book, enable works on existing company
    - ✅ account list: human mode renders without crashing (table import regression)
    - ✅ update: fetches from origin/main via --repo (fixture only, never the live repo)
    - ✅ vat file + settle with a custom af-te-dragen account (--account 2515)
    - ✅ vat file + vat settle end-to-end: filing moves the position, the payment cancels it with the rounding difference in the P&L
    - ✅ entry post --dry-run: rejects non-draft entries instead of a green plan
    - ✅ entry reverse --dry-run: rejects drafts (NOT_POSTED) and double reversals
    - ✅ vat book --dry-run: validates date and description like the execute path

### company.test.js — company show/update

6 passing · 0 failing

    - ✅ company update: sets address/iban/city and audits
    - ✅ company update: dry-run writes nothing
    - ✅ company update: no options -> NOTHING_TO_UPDATE
    - ✅ company update: invalid IBAN rejected
    - ✅ company show: returns the company record
    - ✅ company show: NO_COMPANY on a database without a company row

### direct-debit.test.js — SEPA direct debit: mandate register, pain.008.001.02 export, FRST/RCUR, CORE/B2B split

8 passing · 0 failing

    - ✅ mandates: add/list/remove with audit; guards
    - ✅ direct-debit batch: FRST then RCUR, mandate snapshot on the line
    - ✅ direct-debit batch without a mandate → MANDATE_REQUIRED
    - ✅ payment-term isolation: transfer batch rejects direct-debit payables and vice versa
    - ✅ buildPain008: structure, mandate data, NOTPROVIDED agents, CORE/B2B split
    - ✅ export: DD batch → pain.008.001.02, one export per batch; transfer regression
    - ✅ cli: mandate add/list + direct-debit batch create/export e2e
    - ✅ mcp: mandate add/list + batch create/export (dry-run parity + execute)

### edge-cases.test.js — rounding, boundaries, idempotency, lifecycle violations, dry-run hygiene

35 passing · 0 failing

    - ✅ ledger: unbalanced, zero-amount, too-few postings rejected
    - ✅ ledger: same account on both sides is legal
    - ✅ ledger: reversal guards — draft and double-reversal rejected
    - ✅ ledger: posted entries are immutable — direct UPDATE blocked by trigger
    - ✅ ledger: drafts excluded from balans and P&L
    - ✅ invoice: quantity and price guards
    - ✅ invoice: line parser — Dutch comma price, @ inside description
    - ✅ invoice: line parser — price-only integer price, lowercase vat codes, negative qty (regression)
    - ✅ invoice: per-line rounding edge — 3x 0.01 @21 has 1 cent VAT (line-total rounding)
    - ✅ invoice: 0% and exempt (V) lines book without VAT
    - ✅ invoice: credit note of a paid invoice; credit of a credit rejected
    - ✅ invoice: lifecycle — pay draft rejected, overpayment rejected, overdue derived
    - ✅ invoice: UBL escaping and verlegd category
    - ✅ invoice: due date crosses the year boundary
    - ✅ recurring: day 28 keeps the 28th every month (no drift)
    - ✅ recurring: quarterly and yearly frequencies
    - ✅ recurring: end_date stops the schedule
    - ✅ recurring: templateId run only runs that template even when due later
    - ✅ recurring: depreciation with residual value — final run absorbs the remainder
    - ✅ bank: import is idempotent — same statement twice = 0 duplicates on re-import
    - ✅ bank: Rabo CSV with Af/Bij and Dutch decimals parses correctly
    - ✅ bank: auto-match prefers an exact entry over an invoice for the same amount
    - ✅ bank: partial payment does not auto-match the invoice
    - ✅ vat: mixed rates in one entry, monthly period readout
    - ✅ vat: private use (P) -> 1d/5a at the standard rate (21%)
    - ✅ vat: R income (verlegd binnenland sale) reports the base in 1c, no VAT due
    - ✅ year-end: loss year closes with negative result into equity
    - ✅ year-end: fiscal year end 06-30 drives the jaarrekening as-of date
    - ✅ jaarrekening: custom account lands in Overig, totals still balance
    - ✅ jaarrekening: micro with no activity — zero balans, balanced
    - ✅ year-end: closing two different years works independently
    - ✅ icp: credit note reduces the customer total; period boundary respected
    - ✅ icp: RE base uses the DISCOUNTED amount (agrees with the OB 2a base)
    - ✅ fx: setFxRate with a raw float rate parses as 1.0875, not 1.0875 x10000
    - ✅ all mutating paths leave no trace in dry-run

### entries.test.js — journal entries: add/post/reverse, immutability

17 passing · 0 failing

    - ✅ default chart is seeded with 29 accounts (incl. 4840 Koersverschillen)
    - ✅ createEntry: balanced 2-posting entry lands as draft
    - ✅ createEntry: agent actor is recorded
    - ✅ createEntry: rejects unbalanced postings
    - ✅ createEntry: rejects fewer than 2 postings
    - ✅ createEntry: rejects zero-amount postings
    - ✅ createEntry: rejects unknown and inactive accounts
    - ✅ createEntry: rejects invalid date and missing description
    - ✅ postEntry: draft -> posted, idempotence guarded
    - ✅ postEntry: DB trigger blocks unbalanced drafts
    - ✅ postEntry: DB trigger requires >= 2 postings
    - ✅ postings of a posted entry are immutable (triggers)
    - ✅ reverseEntry: posts linked contra-entry; original stays posted
    - ✅ reverseEntry: guards
    - ✅ parsePostingSpecs: repeatable and comma-separated, negative = credit
    - ✅ every mutation writes an audit record
    - ✅ listEntries: filters

### export.test.js — export xaf (Auditfile 4.0, round-trips through the importer) + audit csv/xlsx

10 passing · 0 failing

    - ✅ export xaf: writes a 4.0 file with header, chart and one Mutatie per posted entry
    - ✅ export xaf: 3-leg entry round-trips through the importer losslessly
    - ✅ export xaf: follows the FISCAL year for non-calendar fiscal years
    - ✅ export xaf: records an export.xaf audit row
    - ✅ export xaf: throws EXPORT_EMPTY_YEAR for a year with no posted entries
    - ✅ export xaf: escaping — ampersands and < in descriptions survive XML
    - ✅ cli: bukio export xaf --year --out writes a file
    - ✅ audit: csv format exports rows with headers
    - ✅ audit: xlsx format requires --out and writes a workbook
    - ✅ export xaf: unknown-year-only drafts → EXPORT_EMPTY_YEAR via CLI

### fiscal-year.test.js — 

7 passing · 0 failing

    - ✅ fiscalYearWindow: year 2026 for FYE 03-31 spans 2025-04-01..2026-03-31
    - ✅ report pnl --year uses the fiscal window (jan prev-FY in, nov this-FY out)
    - ✅ report journal --year uses the fiscal window
    - ✅ report trial-balance --year uses the fiscal window
    - ✅ pnl() module with explicit from/to is untouched by the fiscal change
    - ✅ sales() uses the fiscal window for --year
    - ✅ MCP pnl tool reports the fiscal window

### hardening.test.js — 

85 passing · 0 failing

    - ✅ reversal of a VAT entry cancels the OB readout and keeps vat fields
    - ✅ parsePeriod rejects out-of-range months
    - ✅ dispose at exactly book value (result 0) books a balanced entry
    - ✅ dispose a fully-depreciated asset with no proceeds
    - ✅ invoice create rejects impossible calendar dates
    - ✅ two identical same-day CAMT entries both import (distinct AcctSvcrRef)
    - ✅ bank CSV surfaces skipped rows instead of dropping them silently
    - ✅ payments batch CSV without a header parses positionally
    - ✅ buildDepreciationTemplate rejects a non-positive final run
    - ✅ vat book with @V (vrijgesteld) and @0 (nultarief) books without a zero leg
    - ✅ vat book with @R (verlegd) books NO VAT leg — self-assessed, nets to zero
    - ✅ FX+VAT booking absorbs rounding drift (rate 1.0001, 41.33 USD @21)
    - ✅ FX+VAT: a range of amounts never trips UNBALANCED
    - ✅ CLI: vat book --json reports the vat_code on tagged postings
    - ✅ CLI: invoice pay rejects non-international amounts
    - ✅ dry-run: contact add/update/markPaid write nothing
    - ✅ dry-run: compliance mark / fx set / recurring pause / account reactivate write nothing
    - ✅ dry-run: bank add + link write nothing
    - ✅ CLI: backup --dry-run writes no file
    - ✅ batch delete cascades lines and releases payables
    - ✅ autoMatch never crosses bank accounts that share a ledger code
    - ✅ SEPA MsgId stays within the 35-char limit even for huge batch ids
    - ✅ UBL uses EUR currencyID and carries the supplier postal code
    - ✅ CSV and XLSX exports neuter formula injection
    - ✅ jaarrekening XLSX guards formula injection in account and company names
    - ✅ invoice list --status overdue filters the derived status
    - ✅ MCP: vat_book execute leaves a draft unless post=true; invoice_pay defaults to outstanding
    - ✅ entry with the same account on both sides books the net
    - ✅ reversal of an FX entry negates the fx amounts
    - ✅ invoice finalize with a 0% line books a tagged zero-vat posting
    - ✅ parseAmount boundaries: 1 decimal, zero, negatives, large values
    - ✅ obReadout period with a year boundary stays within the period
    - ✅ CLI: import xaf failure prints cleanly (no renderErrors crash)
    - ✅ CLI: assets register --format csv has a header row and totals
    - ✅ CLI: recurring run --dry-run renders plans, not undefined ids
    - ✅ CLI: export xaf --dry-run writes nothing; scheme/depreciation dry-runs validate
    - ✅ bank ignore dry-run leaves the transaction untouched
    - ✅ assets pause dry-run leaves the status unchanged
    - ✅ autoMatch books a small FX difference on an invoice payment to 4840
    - ✅ paymentFromBank with an FX gain books a credit on 4840
    - ✅ a difference beyond the sanity bound is not an FX move — rejected
    - ✅ 4840 Koersverschillen is created on demand for pre-2026-08-07 databases
    - ✅ paymentFromBank is atomic: a failing entry leaves no payment behind
    - ✅ the FX sanity floor is 25 cents — a 10% short payment on a €10 invoice is rejected
    - ✅ 4840 creation on demand is audited
    - ✅ postFromTransaction is atomic: a failing post leaves no draft or reconciliation
    - ✅ createInvoice rejects non-integer due-days and malformed delivery dates cleanly
    - ✅ createTemplate rejects non-integer due-days for invoice templates
    - ✅ fetchEcbRate rejects a malformed date instead of throwing Invalid time value
    - ✅ importTransactions rejects garbage or impossible transaction dates
    - ✅ createPaymentBatch rejects a garbage batch date (it would land in pain.001)
    - ✅ jaarrekening and exportXaf reject a non-YYYY year instead of building nonsense documents
    - ✅ invoice reminders --within-days 0 stays 0 and garbage is rejected (no silent 7)
    - ✅ list limits validate at the module boundary (INVALID_LIMIT, not SQLITE_MISMATCH)
    - ✅ CLI --limit 0 returns 0 rows; garbage --limit errors (no parseInt || default masking)
    - ✅ MCP: journal honors limit with a truncation flag; year and limit are validated
    - ✅ addPayable rejects garbage or impossible dates (they would land in the payables register)
    - ✅ assets run/register reject garbage periods and as-of dates (no silent over-booking)
    - ✅ import xaf skips a duplicate Boekstuknummer within the same file (parity with AuditFile layout)
    - ✅ recurring run rejects a garbage as-of (it generated 120 draft runs before)
    - ✅ year-end status rejects a non-YYYY year
    - ✅ bank match auto validates --window-days (garbage errors, 0 stays 0)
    - ✅ autoMatch FX tolerance matches the posting tolerance exactly (SQL integer-division drift)
    - ✅ year-end close handles a zero-result year (income == expense) without zero-amount legs
    - ✅ recurring add --due-days 0 stays 0 (the old Number(x) || 30 masked it)
    - ✅ recurring add rejects garbage --due-days (INVALID_DUE_DAYS) instead of silently defaulting to 30
    - ✅ recurring add --day 0 and --day abc are rejected (INVALID_DATE) instead of silently becoming day 1
    - ✅ recurring run dry-run previews due_date = invoice date when due_days is 0 (parity with the real run)
    - ✅ recurring add --dry-run validates like the real run (garbage rejected, nothing written)
    - ✅ invoice create --dry-run validates like the real run (garbage date/contact rejected, nothing written)
    - ✅ creditInvoice dry-run validates like the real run (no plan for nonexistent/unfinalized invoices)
    - ✅ entry add --dry-run validates like the real run (garbage date/desc/unbalanced rejected, nothing written)
    - ✅ entry add rejects day-overflow dates (2026-02-30 was posted before)
    - ✅ import opening-balances rejects a day-overflow --date
    - ✅ fx set rejects a day-overflow date (it used to store 2026-02-30 in fx_rates)
    - ✅ report balans rejects a garbage as-of (it silently read as "forever" before)
    - ✅ report pnl / journal / trial-balance reject a garbage year (no abc-01-01 ranges)
    - ✅ entry list rejects garbage date bounds (--date-to garbage returned ALL entries before)
    - ✅ import opening-balances accepts the documented optional header row (2- and 3-column)
    - ✅ MCP entry_add dry-run validates like execute (garbage date/unbalanced/single-posting rejected, no isError:false plan)
    - ✅ MCP entry_reverse / invoice_credit / invoice_pay dry-runs validate like execute
    - ✅ init validates iban, vat choice and fiscal-year-end (garbage was stored silently)
    - ✅ account add/deactivate/reactivate/import are audited (they mutated silently before)
    - ✅ every emitted error code in src/ is documented in AGENTS.md §7
    - ✅ MCP on a missing database errors NO_DATABASE instead of silently creating an empty company

### import-invoice.test.js — inbound UBL (EN 16931/Peppol) invoice import into payables: idempotent, VAT reported not booked

11 passing · 0 failing

    - ✅ importUblInvoice: registers a payable, matches contact by vat-id, parses VAT
    - ✅ importUblInvoice: idempotent re-import → duplicate skipped
    - ✅ importUblInvoice: --create-missing creates the supplier contact with address + vat-id
    - ✅ importUblInvoice: TaxScheme/cbc:ID is the literal scheme id, not the VAT number
    - ✅ importUblInvoice: explicit --contact wins; no match and no flag → CONTACT_NOT_FOUND
    - ✅ importUblInvoice: validation failures write nothing
    - ✅ importUblInvoice: due date defaults to issue + 30 days
    - ✅ importUblInvoice: dry-run validates like execute but writes nothing
    - ✅ cli: import invoice end-to-end → payable in the register, dry-run plan
    - ✅ mcp: invoice_import dry-run parity + execute
    - ✅ importUblInvoice: multiple PartyTaxScheme entries — VAT number still extracted

### import.test.js — opening balances, journal CSV, XAF (both layouts), contacts — whole-file validation, RGS inference

37 passing · 0 failing

    - ✅ parseImportAmount: international, Dutch comma, thousands-dot forms
    - ✅ opening-balances: imports ONE posted Beginbalans entry (source import)
    - ✅ opening-balances: Dutch code,debet,credit layout
    - ✅ opening-balances: validation collects ALL errors, writes nothing
    - ✅ opening-balances: re-import is rejected
    - ✅ opening-balances: dry-run validates and writes nothing
    - ✅ opening-balances: unknown account and zero amount rejected
    - ✅ journal: one posted entry per boekstuk, two postings per line
    - ✅ journal: idempotent re-import skips existing boekstukken
    - ✅ journal: unknown account fails whole-file validation without --create-missing
    - ✅ journal: --create-missing infers type from net movement
    - ✅ journal: bad amount and date mismatch per boekstuk are both collected
    - ✅ journal: missing required header column rejected
    - ✅ xaf: imports mutaties and creates file-chart accounts
    - ✅ xaf: btw codes are reported, not booked
    - ✅ xaf: idempotent per boekstuknummer
    - ✅ xaf: rekening not in file chart nor chart of accounts -> validation error
    - ✅ xaf: unsupported version rejected
    - ✅ xaf: COMPANY_MISMATCH blocks importing another company
    - ✅ xaf: name mismatch is only a warning
    - ✅ xaf: dry-run validates and writes nothing
    - ✅ xaf (AuditFile layout): imports transaction, creates + renames chart accounts
    - ✅ xaf (AuditFile layout): accounts with postings are NOT renamed
    - ✅ xaf (AuditFile layout): unbalanced transaction fails whole-file validation
    - ✅ xaf (AuditFile layout): idempotent per TransactionID
    - ✅ xaf (AuditFile layout): dry-run lists renames and writes nothing
    - ✅ xaf (AuditFile layout): 8-digit CompanyID mismatch is an error
    - ✅ xaf (AuditFile layout): company name mismatch is only a warning
    - ✅ import contacts: suppliers + customers mapped to contacts
    - ✅ import contacts: idempotent by name
    - ✅ import contacts: entry without a name fails whole-file validation
    - ✅ inferRgs: keywords within type, then type-based fallbacks
    - ✅ import xaf (AuditFile): created accounts carry inferred RGS codes
    - ✅ import xaf: re-import backfills RGS codes on accounts that lack them
    - ✅ import journal: --create-missing accounts also get RGS codes
    - ✅ import chart CSV without an rgs column infers RGS codes
    - ✅ import contacts: dry-run writes nothing

### invoice-features.test.js — 

36 passing · 0 failing

    - ✅ fractional quantities parse to milli-units
    - ✅ line discounts parse (pct and amount)
    - ✅ item specs parse (id, qty, overrides, discount)
    - ✅ allocateLargestRemainder sums exactly and is deterministic
    - ✅ item add/list/show/update/deactivate with audit
    - ✅ item guards: name/unit/price/vat/account
    - ✅ item without a VAT code is allowed when the VAT module is off
    - ✅ unit labels localize
    - ✅ invoice create --items snapshots catalog values
    - ✅ invoice create --items per-invoice overrides (price, VAT, discount)
    - ✅ item guards on invoices: unknown, inactive, bad override, conflicting sources
    - ✅ fractional quantity line math (1.5h @ 100 = 150.00)
    - ✅ line discount pct and amount reduce net and VAT
    - ✅ total discount: single rate, pct and amount
    - ✅ total discount across mixed VAT rates allocates to the cent
    - ✅ total discount with awkward split still balances (largest remainder)
    - ✅ computeInvoiceTotals is deterministic across recomputes (getInvoice consistency)
    - ✅ booking with discounts: omzet uses discounted nets, VAT per rate
    - ✅ finalize with discounts books a balanced entry
    - ✅ invoice language: nl default, en allowed, invalid rejected
    - ✅ CLI: --discount-pct and --discount-amount together are rejected
    - ✅ credit note inherits language, total discount and line discounts
    - ✅ UBL: formatted quantity, unit code, language, discounted tax bases
    - ✅ UBL: line-only discounts net LineExtensionAmount (BR-26); no doc allowance emitted; @V maps to E, @0 to Z
    - ✅ UBL: zero-VAT categories (RE/V) still emit TaxSubtotal — EN 16931 1..n
    - ✅ UBL: hour unit maps to HUR
    - ✅ PDF: Dutch labels, unit column, VAT breakdown, discount row
    - ✅ PDF: English labels + reverse-charge wording
    - ✅ PDF: company logo renders as a data URI in the header
    - ✅ PDF: renders through Chromium (skipped when no browser installed)
    - ✅ recurring invoice template with items snapshots catalog prices per run
    - ✅ MCP: item_add/item_list/item_update + invoice_create with items/discount/language
    - ✅ bank autoMatch: incoming payment matches a DISCOUNTED invoice at its discounted gross
    - ✅ bank autoMatch: discounted invoice does NOT match a partial/off payment
    - ✅ company logo: set (PNG), extract round-trip, remove; audits
    - ✅ company logo: format, size and dimension guards

### invoice.test.js — invoicing: lifecycle, 12-vereisten, credit notes, payments, reminders

21 passing · 0 failing

    - ✅ parseLineSpec: qty, description, price, vat
    - ✅ createInvoice: draft with line math (2x 150 @21 = 300 net, 63 vat)
    - ✅ createInvoice: guards
    - ✅ validateCompliance: 12 vereisten — supplier and customer data required
    - ✅ finalize: assigns sequential number and books Debiteuren/Omzet/btw
    - ✅ finalize: multiple VAT rates -> per-rate postings, exact vat
    - ✅ finalize: VAT module off -> net-only booking, no vat postings
    - ✅ finalize: already finalized is rejected; dry-run writes nothing
    - ✅ credit note: reversed booking, sequence continues
    - ✅ payments: partial then full -> paid; overpayment rejected
    - ✅ nextInvoiceNumber: year-scoped sequence
    - ✅ UBL: Peppol BIS 3.0 structure
    - ✅ UBL: credit note uses CreditNote root + type 381 (Peppol BIS 3.0)
    - ✅ UBL: XML control characters in descriptions are stripped (Peppol-safe)
    - ✅ bank auto-match: incoming payment pays the invoice and posts Bank/Debiteuren
    - ✅ buildInvoicePostings: sales vs credit sign flip
    - ✅ invoiceReminders: overdue + due-soon, excludes paid and far-future
    - ✅ invoiceReminders: within-days controls the due-soon window
    - ✅ invoiceReminders: credit notes are not reminder candidates
    - ✅ validateCompliance: VAT-exempt company without btw-id can still invoice
    - ✅ validateCompliance: VAT company without btw-id still fails SUPPLIER_INCOMPLETE

### money.test.js — integer-cents money helpers

5 passing · 0 failing

    - ✅ parseAmount: valid inputs
    - ✅ parseAmount: rejects invalid inputs
    - ✅ parseAmount: rejects more than 2 decimals
    - ✅ formatAmount: round-trips with parseAmount
    - ✅ formatAmount: formatting

### month-end.test.js — month-end close check

8 passing · 0 failing

    - ✅ month-end: clean month -> all clear, zero totals
    - ✅ month-end: drafts and unmatched bank transactions are flagged
    - ✅ month-end: VAT quarter readout when module on
    - ✅ month-end: profit = income - expense for the period
    - ✅ month-end: December totals exclude year-end closing entries
    - ✅ month-end: overdue invoice warning with outstanding total
    - ✅ month-end: invalid period rejected
    - ✅ month-end: draft invoices are warned (booked revenue may be uninvoiced)

### payments.test.js — SEPA payment batches: payables, pain.001 export

21 passing · 0 failing

    - ✅ isValidIban: mod-97 check with normalization
    - ✅ payables: add transfer + direct-debit, audit, list filters
    - ✅ payables: unknown contact, bad amount, missing ref rejected
    - ✅ payables: mark paid (dry-run writes nothing, real is audited)
    - ✅ contacts: iban on create (validated) and update (audited)
    - ✅ batch: explicit lines resolve contacts by name or id
    - ✅ batch: company without IBAN fails with a hint
    - ✅ batch: contact without IBAN fails with a hint and details
    - ✅ batch: invalid iban, zero amount, missing name, long reference all collected
    - ✅ batch: SEPA names longer than 70 chars are rejected (Max70Text)
    - ✅ batch: payables path rejects a contact name longer than 70 chars
    - ✅ batch: company name longer than 70 chars is rejected
    - ✅ batch: from payables excludes direct-debit and marks payables in_batch
    - ✅ batch: dry-run writes nothing; empty batch rejected
    - ✅ batch CSV: comma and semicolon delimiters, Dutch amounts
    - ✅ batch CSV: whole-file validation reports every bad line
    - ✅ export: pain.001.001.03 XML with totals, SEPA level, escaping; re-export blocked
    - ✅ export: escaping and .09 schema
    - ✅ buildPain001: batch date lands in ReqdExctnDt
    - ✅ delete: only drafts; payables released back to unpaid
    - ✅ getPaymentBatch: serializes total + lines

### recurring-invoice.test.js — subscription invoice templates

8 passing · 0 failing

    - ✅ invoice template: generates draft invoices on schedule (never auto-finalizes)
    - ✅ invoice template: generated drafts finalize normally (compliance + number)
    - ✅ invoice template: guards
    - ✅ invoice template: entry templates keep working alongside
    - ✅ invoice template: dry-run shows the invoice plan, writes nothing
    - ✅ invoice template: runs limit completes the template
    - ✅ peppol send: posts the UBL to the provider (mock server)
    - ✅ peppol send: not configured / provider error / dry-run

### recurring.test.js — recurring entries engine: schedules, depreciation, accruals

21 passing · 0 failing

    - ✅ addPeriod: monthly/quarterly/yearly with day preserved
    - ✅ createTemplate: validates postings, balances, accounts
    - ✅ createTemplate: inactive account rejected
    - ✅ createTemplate: first run normalized to day_of_period
    - ✅ runDue: books one entry per period on schedule
    - ✅ runDue: idempotent — nothing due means nothing generated
    - ✅ runDue: runs limit completes the template
    - ✅ runDue: end_date completes the template
    - ✅ runDue: paused templates are skipped
    - ✅ runDue: --template runs only that template
    - ✅ runDue: dry-run writes nothing
    - ✅ reverse_previous: accrual pattern — each run reverses the prior entry
    - ✅ reverse_previous: completed accrual chain nets zero after final run
    - ✅ reverse_previous: dry-run preview mirrors the execute shape (reversal + new entry)
    - ✅ buildDepreciationTemplate: remainder-adjusted final run, cents-exact total
    - ✅ buildDepreciationTemplate: validation
    - ✅ vat-aware template: expansion stored, generation replays it
    - ✅ vat-aware template: requires VAT module on
    - ✅ previewDue: read-only plan matches runDue
    - ✅ listTemplates: status filter
    - ✅ generated entries are immutable + trial balance stays balanced

### reports-v014.test.js — aging buckets, contact statements, sales analytics (by contact/item)

10 passing · 0 failing

    - ✅ aging debtors: buckets, totals, paid excluded, contacts sorted by total
    - ✅ aging debtors: finalized credit notes reduce the outstanding, drafts do not
    - ✅ aging creditors: buckets + in_batch shown separately
    - ✅ aging validation: bad as-of and kind rejected
    - ✅ contact statement: running balance ends at outstanding; supplier side negative
    - ✅ contact statement: credit notes reduce the balance (regression)
    - ✅ sales by contact: net/vat/gross from the totals engine; credit notes excluded
    - ✅ sales by item: catalog items group by item_id, ad-hoc lines by description
    - ✅ cli: report aging + sales + contact statement e2e with csv export
    - ✅ mcp: report_aging and report_sales expose the same shapes

### reports.test.js — balans, P&L, journal

9 passing · 0 failing

    - ✅ balans: assets = liabilities + equity + result
    - ✅ balans: before any income/expense, result is zero
    - ✅ balans: empty books balance at zero
    - ✅ balans: drafts excluded, reversal nets out
    - ✅ pnl: revenue, costs and result
    - ✅ pnl: empty period gives zero result and no sections
    - ✅ pnl: legacy chart without RGS codes still splits revenue/costs by type
    - ✅ pnl: catch-all section for accounts with unknown rgs_code
    - ✅ journal: one row per posting, ordered by date

### review-round3.test.js — 

3 passing · 0 failing

    - ✅ recurring pause --dry-run and resume --dry-run render a plan (no fmtTemplate crash)
    - ✅ audit --format json prints JSON even without the global --json flag
    - ✅ bank match post --dry-run rejects an already-matched transaction and a missing account

### smtp.test.js — zero-dependency SMTP client + invoice email: auth, STARTTLS, MIME/PDF attachment, dry-run, audit

15 passing · 0 failing

    - ✅ sendMail: happy path delivers, captures the MIME with the PDF attachment
    - ✅ sendMail: auth failure → SMTP_AUTH_FAILED
    - ✅ sendMail: rcpt rejection → SMTP_SEND_FAILED with server text
    - ✅ sendMail: STARTTLS advertised but rejected → SMTP_CONNECT_FAILED (branch exercised)
    - ✅ sendMail: connection refused → SMTP_CONNECT_FAILED; bad greeting → SMTP_CONNECT_FAILED
    - ✅ smtpConfig/smtpValidate: env-driven; missing host/from → SMTP_NOT_CONFIGURED
    - ✅ buildMime: non-ASCII subject → UTF-8 encoded-word; attachment boundary present
    - ✅ buildMime: CR/LF in to/subject/filename cannot inject headers
    - ✅ sendMail: dot-stuffed payload — a body line starting with "." survives
    - ✅ emailInvoice: delivers to the contact email and audits
    - ✅ emailInvoice: guards — draft, missing email, unconfigured SMTP
    - ✅ emailInvoice: dry-run renders the plan, makes no connection, audits nothing
    - ✅ emailInvoice: PDF attachment is rendered and decodes to %PDF
    - ✅ cli: invoice email e2e with SMTP env + audit row
    - ✅ mcp: invoice_email dry-run parity (no connection) + execute

### trial-balance.test.js — trial balance invariants

3 passing · 0 failing

    - ✅ trial balance: startkapitaal + expense, per-account totals
    - ✅ trial balance: year filter
    - ✅ trial balance: drafts are excluded, reversals net out

### update.test.js — 

11 passing · 0 failing

    - ✅ update plan: a non-clone directory is refused
    - ✅ update plan: a non-official remote is refused
    - ✅ update plan: a URL embedding the official path as substring is refused (anchored regex)
    - ✅ update plan: shows the incoming commit and current version without warning
    - ✅ update plan: local modifications are reported as overwrite warnings
    - ✅ update: refuses to run without --yes
    - ✅ update: --yes resets the working tree to origin/main
    - ✅ update: --yes overwrites a local customization (tracked modification)
    - ✅ update: --yes drops local commits (warned in the plan)
    - ✅ update: reinstalls dependencies when package.json changed
    - ✅ update: records an audit row when a company db exists

### vat-settle.test.js — 

18 passing · 0 failing

    - ✅ vat file: owe — 2500 cleared, 2510 holds the exact-cents liability, audited
    - ✅ vat file: refund position — 1500 cleared, 2510 debit (te ontvangen)
    - ✅ vat file: nothing to file when the position is zero
    - ✅ vat file: dry-run writes nothing and does not create the account
    - ✅ vat settle: rounding in your favour (paid less than booked) books the gain to 4700
    - ✅ vat settle: refund received in your favour (more than booked) books a gain
    - ✅ vat settle: paying MORE than booked books a loss to the difference account
    - ✅ vat settle: difference beyond €5 is rejected as the wrong amount
    - ✅ vat settle: nothing to settle without a filed balance
    - ✅ vat settle: direction guard — incoming tx cannot pay a te-betalen balance
    - ✅ vat settle: invalid difference account is rejected
    - ✅ vat settle: dry-run books nothing and leaves the tx unmatched
    - ✅ vat settle: custom difference account (e.g. dedicated Afrondingsverschillen)
    - ✅ vat file + settle round-trip: readout 5d agrees with the booked net position
    - ✅ vat file: 2510 taken by another account falls to the next free code (2511)
    - ✅ vat file: custom account 2515 is used when requested and settle cancels it
    - ✅ vat file: dry-run plans the next free code without creating anything
    - ✅ vat settle: an af-te-dragen account already reused from an earlier filing is settled on its own code

### vat.test.js — optional VAT module: codes, vat book, OB readout 1a–5d

13 passing · 0 failing

    - ✅ enableVatModule: flag, accounts 1500/2500, 8 codes; idempotent
    - ✅ enableVatModule: refuses on KOR company
    - ✅ parseVatPostingSpecs: CODE:AMOUNT[@VATCODE]
    - ✅ expandVatPostings: adds VAT leg, computes vat amount
    - ✅ expandVatPostings: input side goes to 1500 te vorderen
    - ✅ bookVatEntry: posts a 3-leg entry with vat fields persisted
    - ✅ bookVatEntry: guards — module off, unknown code
    - ✅ parsePeriod: quarters and months
    - ✅ obReadout: full scenario fields 1a-5d
    - ✅ obReadout: period isolation and drafts excluded
    - ✅ obReadout: reverse charge fields 3a/4a (nets out via 5b)
    - ✅ obReadout: guards — module off, invalid period
    - ✅ markFiled: records the filing and its fields

### year-end.test.js — annual close, jaarrekening micro/klein, ICP

19 passing · 0 failing

    - ✅ year-end close: posts closing + appropriation, balanced, source closing
    - ✅ year-end close: guards — drafts block, empty year reports
    - ✅ year-end close: dry-run writes nothing
    - ✅ P&L still shows the year result after closing
    - ✅ jaarrekening: klein model — statutory balans + W&V, balanced
    - ✅ jaarrekening: klein model — resultaat counts inkoop ONCE and adds overige bedrijfsopbrengsten
    - ✅ jaarrekening: after closing, result sits in equity (no onverdeeld)
    - ✅ jaarrekening: klein P&L follows the FISCAL year, not the calendar year
    - ✅ year-end close: follows the FISCAL year for non-calendar fiscal years
    - ✅ jaarrekening: invalid model rejected
    - ✅ jaarrekening: account-level amounts are numbers, never NaN
    - ✅ jaarrekening: PDF html renders account detail without NaN
    - ✅ jaarrekening: pnl includes the Afschrijvingen line for WAFS.41
    - ✅ jaarrekening PDF: renders (playwright)
    - ✅ OB readout: R purchase -> 3a/4a, RE purchase -> 3b/4b, RE sale -> 2a
    - ✅ OB readout: verlegde EU sale (RE invoice) reports 2a
    - ✅ ICP readout: EU customers with RE lines, totals per customer
    - ✅ ICP readout: missing customer vat-id fails loudly
    - ✅ ICP readout: no RE lines -> empty listing

---
_Regenerated automatically on every `npm test`._
