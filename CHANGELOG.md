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
- Actor identity Tier 0, task 6: schema migration 018 — per-company
  `actor_keys` registry (Ed25519 public keys, revoked rows retained for
  history) + `settings` table with the `signing_enforce` flag, and six
  audit-log signature columns (`digest_hash`, `sig_keyid`, `sig_nonce`,
  `sig_ts`, `sig`, `sig_status`). Additive only: legacy audit rows read back
  with `sig_status = 'unsigned'` (claimed, not yet provable).
- Actor identity Tier 0, task 3: `src/core/actor-registry.js` — DB-backed
  per-company actor key registry (`actor_keys` + `settings` via migration
  018): enrol (duplicate active key rejected), revoke (row retained with
  reason — history preserved), re-enrol after revocation as the rotation
  flow, `canAct` check, and the per-company `signing_enforce` flag
  (default off).
- Actor identity Tier 0, task 4: `bukio actor` CLI — `keygen` (Ed25519
  keypair; human keys passphrase-encrypted via `BUKIO_SIGNING_PASSPHRASE` or
  interactive prompt, agent/system keys plain files, 0600/0700 permissions,
  `--force` to rotate), `register` (enrol the local key into the current
  company DB, audited), `list`, `revoke --reason` (row retained for
  history, audited), `enforce --on|--off` (per-company, audited),
  `unlock`/`lock` (short-lived session keys, default 12 h,
  `--ttl-hours`), and `verify`. Key/session paths honour `BUKIO_CONFIG_DIR`
  (default `~/.bukio/`). All commands support `--json` and `--dry-run`;
  `sign.js` gains `publicKeyFromPrivate` + `decryptPrivateKey`.
- Actor identity Tier 0, task 5: **the sign-and-verify gate** — every CLI
  command is now digitally signed by its declared actor and verified against
  the per-company registry before dispatch. `src/cli/util.js` gains
  `signCommand`/`verifySignatureBundle`: canonical digest (Task 2) →
  auto-sign via `--sign-key`, session key, `BUKIO_SIGNING_PASSPHRASE` or the
  actor key file → verify against `actor_keys` with a ±5 min timestamp
  window and a 24 h nonce cache (`<config>/nonces.json`, replay refused in
  every mode). `record` mode (default) is backwards compatible: unsigned or
  unverifiable commands run and are logged `sig_status = unsigned`;
  `enforce` mode refuses with `SIGNATURE_REQUIRED` / `SIGNATURE_INVALID` /
  `SIGNATURE_STALE` / `NONCE_REUSED` / `ACTOR_KEY_UNKNOWN` /
  `ACTOR_KEY_REVOKED` before anything mutates (`--dry-run` fails
  identically). Bootstrap commands (`actor keygen`/`unlock`/`lock`) and the
  `actor enforce --off` recovery hatch are exempt; audit rows carry
  digest/signature/keyid/nonce/timestamp via `audit.setPendingSignature`.
- Actor identity Tier 0, task 7: **`bukio audit verify`** — re-verifies the
  signed audit trail against the company registry. `src/audit/index.js`
  gains `verifyTrail()`: recompute the canonical digest from the stored
  signed args (+ the signed command path, now stored on the row) and
  re-check every signature. Per-row `ok | unsigned | revoked (valid at the
  time, since revoked) | tampered | invalid-signature | unknown-key` with
  summary counts; exit 1 on tampered/invalid/unknown-key. Self-contained:
  a copied DB file verifies with no external files. Signed rows now store
  the EXACT signed args + command path in `args_json`/`command` so the
  digest is recomputable. Migration 019: `actor_keys` gains a composite
  `(actor, keyid)` primary key — revoked keys are RETAINED as history, so
  audit rows signed by a rotated key stay verifiable (`getKeyByKeyid`);
  `actor list` shows the full key history.
- Actor identity Tier 0, task 8: **MCP signed execution** — every mutating
  MCP tool call is signed by its actor before the handler runs, sharing the
  CLI's gate via a new `signPayload()` helper in `src/cli/util.js` (the CLI's
  `signCommand` now delegates to it, so the digest scheme, ±5 min window and
  nonce cache are byte-identical across both surfaces). Payload: `cmd =
  mcp:<tool_name>`, args = the tool arguments minus the identity flag;
  enforcement state of the company DB applies equally (unsigned/unverifiable
  calls refused with the same error codes as the CLI, before any mutation or
  plan). Read-only tools are not gated. The `mcp` command itself is exempt
  from the CLI preAction gate — it mutates nothing and must be able to START
  under enforce; the per-tool gate is what enforces. `audit verify` accepts
  MCP-signed rows (verified cross-surface).
- Actor identity Tier 0, task 9: **lifecycle coverage + documentation** —
  one end-to-end scenario test walks a real enrolment: keygen (agent plain,
  human passphrase-encrypted) → unlock → register → enforce on → signed
  commands run (both actors) → unsigned refused → lock (passphrase required
  again) → revoke → rotation (keygen --force + register) → `audit verify`
  clean (old rows `revoked`, new rows `ok`, zero tampered/invalid/unknown)
  → company B independence (same key: refused until enrolled, then verified;
  B's fresh registry shows no leaked revocation). README gains the "Actor
  identity & signing" section, `bukio actor` + `bukio audit verify` command
  references, and signing rows in the global flags table; AGENTS.md gains
  house rules 10–13 (enforcement, per-company enrolment, human sessions,
  cron/system keys); the README test badge is synced to the live count.
- Actor identity Tier 0, security tightening (review follow-up): **`actor
  register` is no longer blanket-exempt from signing.** The exemption is now
  limited to *rotation re-enrolment* — an actor whose key was revoked (no
  active key) may register under enforcement, because that is the only way
  back in. **First-time enrolment under enforcement is refused**
  (`ACTOR_KEY_UNKNOWN`, with a message pointing at the audited onboarding:
  `enforce --off` → `register` → `enforce --on`), so a brand-new actor
  identity cannot be self-registered behind an enforced company — enrolment
  stays an operator-gated, audited act. Lifecycle test extended: company B
  asserts the refusal and the full operator-gated onboarding.
- Actor identity Tier 0, security tightening (review follow-up 2): **`actor
  enforce --off` is no longer exempt either** — only an *enrolled* actor
  with a valid signature may disable enforcement (`ACTOR_KEY_UNKNOWN` /
  `SIGNATURE_REQUIRED` for everyone else). This closes the last unautho-
  rised path into an enforced company (off → keygen → register → on).
  Accepted consequence, documented in README + AGENTS.md: if *all* enrolled
  keys are lost, the CLI is locked out on that company (recover via backup,
  or the owner edits the DB directly). Tests updated: the enforce toggle
  test enrols the operator first; the exempt-commands test now asserts the
  refusal and the enrolled-actor path; the lifecycle test's company B walks
  the full operator-gated onboarding.
- Actor identity Tier 0, docs clarity (review follow-up): the README and
  AGENTS.md now walk through **what an actor actually does** — one-time
  setup per identity (`actor keygen` + `actor register` per company DB),
  per-session `actor unlock` for humans, then fully automatic signing of
  every command (reads, writes, `--dry-run`): canonical digest → signature
  → registry verification → audit row `verified`. AGENTS.md gains worked
  example 6.20 with the refusal→fix table (`SIGNATURE_REQUIRED` /
  `ACTOR_KEY_UNKNOWN` / `ACTOR_KEY_REVOKED` / `PASSPHRASE_REQUIRED`);
  README gains an "In practice — what you actually do" block; §3 quick
  reference rows for `actor register` / `actor enforce` aligned with the
  operator-gated enrolment and enrolled-only `--off`.
- **Actor authorizations Tier 0.5** (per-actor segregation of duties,
  builds on Tier 0 signing): capability families + roles instead of a
  per-command matrix.
  - Migration 020: `actor_roles` table (actor, role with a CHECK on
    `owner|bookkeeper|payments|tax|assets|readonly`, granted_by,
    granted_at) + the per-company `authz_mode` setting; the role→
    capability map lives in code (`src/core/authz.js`), not the DB.
  - `src/core/authz.js`: 20 capability families, role→capability map,
    command→capability for the full CLI surface + every MCP mutating
    tool (`entry add --post` → `entry.post` via the actual mutation),
    `canAct` (fail-closed: unmapped commands deny), SoD conflict-pair
    warnings (`bookkeeper`+`payments`, `bookkeeper`+`tax`,
    `payments`+`tax`, `entry.post`+`payments.sepa`; the owner role is
    exempt), the exemption set (self-service: register/verify/own
    roles/can/self-revoke + the signing-exempt commands), and
    `checkAuthz` — the gate that runs in `signPayload` after signature
    verification, refusing with `AUTHZ_DENIED` before anything is
    written (dry-run included).
  - CLI: `actor authz --on|--off` (implies signing enforcement; the
    flipper becomes owner — bootstrap without deadlock; `--off` needs
    the owner role), `actor roles grant/revoke <role> --for <who>`
    (grants warn softly on SoD conflicts; the LAST owner can never be
    revoked — `LAST_OWNER`), `actor roles [--for <who>]` (self-service
    for your own roles), `actor can '<cmd>' [--for <who>]` (capability
    check, actual-mutation aware), `actor who-can '<cmd>'` (the SoD
    review matrix), and `actor revoke --target <who> --reason` — the
    owner-mediated key kill, owner role required regardless of authz
    mode. Note: the memo's `--actor <who>` grantee flag became
    `--for <who>` — the identity flag `--actor` is the root commander
    option and cannot be re-declared on a subcommand (commander would
    silently bind the value to the root and corrupt the signing
    identity).
  - MCP: mutating tool calls gate through the same `signPayload`
    capabilities (`entry_add post:true` → `entry.post`; read-only tools
    are not gated); fixed a Tier 0 gap — the MCP server now honours
    `BUKIO_ACTOR` env (was hardcoded `agent:mcp`, which broke signing
    under enforcement for env-driven sessions).
  - `buildSignedArgs` keeps meaningful `--for`/`--target` options in the
    signed payload (the grant target is tamper-evident on the audit
    row).
  - Docs: README "Authorizations & segregation of duties" section +
    `actor` command table rows + roadmap row; AGENTS.md §3 rows, §7
    error codes (`AUTHZ_DENIED`, `INVALID_AUTHZ`, `INVALID_ROLE`,
    `ROLE_NOT_GRANTED`, `LAST_OWNER`) and the §6.21 walkthrough.
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
