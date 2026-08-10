# Changelog

All notable changes to **bukio-cli** are recorded in this file. The format
loosely follows [Keep a Changelog](https://keepachangelog.com/); versions
match `package.json` and are bumped at release time. Work in progress on the
`dev` branch lives under **[Unreleased]** and moves to a version heading when
merged to `main` and released.

## [Unreleased] — dev branch

### Added

- Actor identity Tier 0, task 1: `src/core/sign.js` — Ed25519 keypair
  generation (plain and passphrase-encrypted PKCS8 via aes-256-cbc), signing
  and verification (RFC 8032, `node:crypto` only — no new dependencies), and
  a stable keyid fingerprint (`sha256` of the SPKI DER bytes, first 16 bytes
  as 32 hex chars). Signed commands and the `audit verify` trail build on
  this module in later tasks.
- Actor identity Tier 0, task 2: `src/core/canonical.js` — deterministic
  sha256 digest of the canonical command shape `{ v, actor, cmd, args, ts,
  nonce }` (sorted-key JSON; identity/output flags `--actor`, `--sign-key`,
  `--json` are excluded from the signed args, `--dry-run` is included).
  Every signed command signs this digest; `audit verify` recomputes it from
  the stored args to detect tampering.
- `test/company-simulation.test.js`: full-year end-to-end simulation of one
  fictitious company driven through the real CLI — sales with line/total
  discounts, mixed VAT rates, EU reverse charge, credit notes; purchases
  (standard / EU verlegd / binnenlands verlegd); CAMT bank import + auto-match;
  two complete OB readout → `vat file` → `vat settle` cycles; reports; SEPA
  payables; year-end close + jaarrekening + ICP. Exact-cents assertions, the
  trial balance must stay balanced after every stage (12 tests).
- `CHANGELOG.md`: this file — the version history is now recorded here.

### Changed

- `bukio report balans` → **`bukio report balance-sheet`**. The Dutch spelling
  remains available as a deprecated alias (commander `.alias('balans')`) so
  existing scripts and cron jobs keep working. The MCP tool is renamed
  `balans` → `balance_sheet`; XLSX sheet name and human-mode header now read
  "Balance Sheet". The statutory Dutch term in the jaarrekening ("Balans per
  …") and the XAF `RekeningSoort` value `Balans` are official format names and
  were left untouched.

## [0.14.1] — 2026-08-09

### Added

- `vat file` + `vat settle`: the af-te-dragen omzetbelasting flow — filing
  reclassifies the net 1500/2500 position to a separate liability account
  (default 2510, auto-created), the later bank payment cancels it, and the
  whole-euro filing vs exact-cents rounding difference lands in a P&L
  difference account (default 4700). Account-collision fallback to the next
  free numeric code + custom `--account`/`--difference-account`.
- `bukio update`: self-update from the GitHub main branch (dry-run first,
  `--yes` confirmation, audit row, `--trust-remote` for forks).
- OB readout reports domestic reverse-charge sales (`R`) in field 1c; UBL
  reverse-charge tax percent uses the code's configured rate.

### Fixed

- 22 review rounds across the codebase, including: reverse-charge VAT no longer
  corrupts the 2500 balance; UBL/Peppol BIS 3.0 compliance (mandatory
  `DocumentCurrencyCode` BT-5, BuyerReference BT-10, credit-note billing
  reference BT-25, Peppol EndpointID BT-34/BT-49, TaxSubtotal coverage for
  zero-VAT categories); autoMatch double-match parameter-order bug and
  discounted-outstanding matching; fiscal-year windows in 5 reports; MCP server
  robustness (non-object messages, schema-required fields, FX dry-run no longer
  stores ECB rates, derived overdue status); bank parser hardening (IBAN dash
  normalization, delimiter decided once); CSV delimiter consistency;
  opening-balances reversal retry; version-drift guard; contact IBAN normalizer
  parity; jaarrekening P&L window + esc quotes; SEPA name limits; disposal
  atomicity; `account list` human-mode crash; dead imports.

## [0.14.0] — 2026-08-08

### Added

- In-database document attachments (`attach add/list/show/remove`, 25 MB cap,
  sha256 dedupe, BLOB or file store, MCP tools).
- AES-256-GCM encrypted backups with keep-N rotation and audited restore.
- Aging report (debtors/creditors, 30/60/90+ buckets, credit-note FIFO
  netting), per-contact statements (opgave), sales analytics by contact/item.
- Inbound e-invoice intake: `import invoice` (EN 16931 / Peppol BIS 3.0 UBL →
  payables register, idempotent, VAT reported but not booked).
- SMTP `invoice email` with a zero-dependency client (auth, STARTTLS, MIME/PDF
  attachment, dry-run, audit).
- SEPA direct debit: mandate register + pain.008.001.02 export (FRST/RCUR,
  CORE/B2B split).

### Changed

- RGS codes mandatory on chart import; P&L grouped by account type.
- Fresh COCOMO benchmark (20.96 KLOC) + AI cost transparency snapshot.

### Fixed

- Review passes 1–2 (3 high, 6 medium, ~15 low): XLSX formula-injection guard,
  XML control-character stripping, crash-idempotent migrations, supplier BT-48,
  MCP `contact_add` dry-run parity, autoMatch cross-account matching, SEPA
  MsgId length guard, and more.

## [0.13.0] — 2026-08-08

### Added

- Items catalog (`item add/list/show/update`, price/VAT snapshots onto invoice
  lines, deactivation).
- Fractional quantities (milli-units) and line discounts (`@-10%` / `@-25.00`).
- Invoice-level discounts (`--discount-pct` / `--discount-amount`) with
  proportional VAT allocation across rate groups (largest remainder) — OB
  readout, jaarrekening, XAF and UBL all reconcile to the cent.
- VAT breakdown per rate on the invoice; `nl`/`en` invoice language; company
  logo (BLOB, travels with backups).
- Recurring item templates + MCP item tools.

### Fixed

- UBL allowance/document-level charge handling (BT-108), VAT category mapping,
  discount-conflict rejection, version references.

## [0.12.0] — 2026-08-07

### Added

- `export xaf`: Auditfile Financieel 4.0 XML for external advisors (round-trips
  through the importer); audit log as csv/xlsx.
- Test report: `test/report.md` lists every test with its result.

### Fixed

- Hardening pass — 12 edge-case bugs with 20 regression tests; three review
  passes (UBL, export injection, derived status, MCP); FX-difference booking on
  invoice payments + bank-path atomicity; clean `INVALID_DATE` errors for
  batch/recurring/invoice/ECB dates; non-YYYY year rejection in jaarrekening
  and XAF export.

### Performance

- Lazy-load playwright-core + exceljs: CLI startup 1.7s → 0.2s, full test suite
  5m25s → 41s.

## [0.11.1] — 2026-08-06

### Changed

- Named actors required: `--actor '<role>:<name>'` (bare `human`/`agent`
  rejected) — every mutation lands in the audit trail with a named actor.

## [0.11.0] — 2026-08-06

### Added

- SEPA payment batches: payables register (`payments payables add/list/pay`),
  batch create/export/delete, pain.001 (`001.03`/`001.09`) with a unique MsgId
  and a once-per-batch export guard, contact IBANs mod-97 validated.

## [0.10.1] — 2026-08-06

### Changed

- RGS codes are now mandatory on chart import (inference for legacy charts);
  P&L driven by account type so imported charts without RGS codes stay correct.

## [0.10.0] — 2026-08-06

### Added

- Fixed assets module: depreciation schemes (lineair / degressief, 5y/0%
  default), mid-life adoption (recognition-date + cum-dep), idempotent monthly
  runs, activastaat register, disposal (sale/scrap), pause/resume, MCP tools.

## [0.9.0] — 2026-08-06

### Added

- Imports: opening balances, SnelStart/Exact-style journal CSV, XML Auditfile
  Financieel 4.0 (Belastingdienst + generic layouts), contacts — all
  whole-file validated and idempotent.
- `month-end --period`: read-only close check (drafts, unmatched bank, OB
  readout, overdue invoices, due recurring, balanced totals, profit).
- Invoice reminders (`invoice reminders --within-days N --draft-emails`).
- `company show/update` (audited, IBAN validation); VAT-exempt invoicing
  without a company btw-id.

## [0.8.0] — 2026-08-05

### Added

- Agent layer: MCP server over stdio (JSON-RPC 2.0, plan-only mutations,
  READONLY mode), compliance calendar (`compliance status`/`mark`).
- FX: rate store (`fx set`/`fx fetch` from the ECB), foreign-currency booking
  (`--currency`/`--rate`), `fx_currency`/`fx_amount_cents` on postings.

## [0.7.0] — 2026-08-05

### Added

- Jaarrekening: year-end close (result → 9900 → 3000, source `closing`,
  P&L stays visible), statutory annual accounts micro/klein (KVK deposit
  package as PDF, XLSX for the accountant), ICP readout for EU verlegde
  leveringen.

## [0.6.0] — 2026-08-05

### Added

- Recurring invoices (draft-only generation; finalization is a separate audited
  action) and Peppol e-invoicing (UBL 2.1 / Peppol BIS 3.0 POST via
  `BUKIO_PEPPOL_ENDPOINT`/`BUKIO_PEPPOL_TOKEN`).

## [0.5.0] — 2026-08-04

### Added

- Invoicing: lifecycle (draft → finalize → sent → paid), 12 factuurvereisten
  validation, PDF (Chromium), UBL, credit notes (reversal entries), payments,
  reminders.

## [0.4.0] — 2026-08-04

### Added

- Recurring entries engine: templates, depreciation helper (`depreciation add`,
  cents-exact, remainder-adjusted final run), accruals with auto-reversal,
  idempotent backfilling, pause/resume, `source='recurring'` audit trail.

## [0.3.0] — 2026-08-04

### Added

- Bank module: CAMT.053 XML + Dutch bank CSV import (Rabo/ING/ABN aliases,
  `1.234,56` amounts, Af/Bij signs), idempotent hashing, auto-match
  (exact ≤ 2d / fuzzy ≤ 5d) / suggest / link / post reconciliation.
- Optional VAT module: accounts 1500/2500, 8 VAT codes (21/9/0/V/R/RE/M/P),
  `vat book` with `@CODE` expansion, OB-aangifte readout fields 1a–5d for
  manual filing, KOR guard.

## [0.2.0] — 2026-08-04

### Added

- Chart of accounts CRUD + CSV chart import; 28-account RGS-mapped chart;
  reports (trial balance, balance sheet, P&L, journal) with CSV/XLSX export;
  backup/restore via the SQLite backup API.

## [0.1.0] — 2026-08-04

### Added

- Core ledger: `init` (company + chart), journal entries (draft → posted →
  reversed with linked contra-entries), posting invariants enforced by DB
  triggers, append-only audit log with actor attribution, `trial-balance`
  report, `--json`/`--dry-run`/`--actor` contract.

---

_Kept up to date on every change — see the bukio-cli-development skill for the changelog convention._
