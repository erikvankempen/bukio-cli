# bukio-cli — test report

**Latest run:** 2026-09-10 18:05:36 UTC — **✅ 469 passing · 0 failing (469 tests)**
**Command:** `npm test` (per-file `node --test --test-reporter=tap`)

## All tests

### actor.test.js — named-actor enforcement, actor identity CLI + sign-and-verify gate (record/enforce modes, stale/replay/registry refusals) + full Tier 0 lifecycle (enrol→enforce→lock→revoke→rotate→verify→company B)

33 passing · 0 failing

    - ✅ isValidActor: role:name formats
    - ✅ actorError: helpful messages for missing and malformed actors
    - ✅ CLI: missing actor fails with ACTOR_REQUIRED
    - ✅ CLI: bare role without a name is rejected (INVALID_ACTOR)
    - ✅ CLI: named actor works; JSON error shape on --json
    - ✅ CLI: BUKIO_ACTOR env satisfies the requirement
    - ✅ CLI: BUKIO_ACTOR env is recorded in the audit trail
    - ✅ actor keygen: agent key writes a plain 0600 key file (BUKIO_CONFIG_DIR respected)
    - ✅ actor keygen: human key is passphrase-encrypted via BUKIO_SIGNING_PASSPHRASE
    - ✅ actor keygen: refuses to overwrite; --force replaces (rotation)
    - ✅ actor keygen: human key without a passphrase in a non-interactive shell fails PASSPHRASE_REQUIRED
    - ✅ actor register: enrols the local key into the current company DB and audits it
    - ✅ actor revoke: requires a reason; revoke marks the row and audits it
    - ✅ actor enforce: --on/--off toggles the per-company flag and audits it
    - ✅ actor unlock: wrong passphrase -> PASSPHRASE_INVALID; correct -> session key with expiry; lock clears it
    - ✅ actor unlock: agent keys are not unlocked per session
    - ✅ actor list: shows enrolled and revoked actors
    - ✅ actor verify: reports key state against the current company registry
    - ✅ actor commands reject invalid actor strings with INVALID_ACTOR
    - ✅ readSessionKey: expired or missing session files count as locked
    - ✅ sign gate: record mode + enrolled key -> command runs, audit row verified
    - ✅ sign gate: record mode + no key -> runs, logged unsigned
    - ✅ sign gate: enforce on + no key -> SIGNATURE_REQUIRED, nothing mutated (JSON contract)
    - ✅ sign gate: enforce on + wrong key -> SIGNATURE_INVALID
    - ✅ sign gate: locked human key -> PASSPHRASE_REQUIRED; env passphrase unlocks
    - ✅ sign gate: unknown actor key -> ACTOR_KEY_UNKNOWN
    - ✅ sign gate: revoked key -> ACTOR_KEY_REVOKED
    - ✅ sign gate: --dry-run fails identically before any mutation
    - ✅ sign gate: keygen stays exempt under enforcement; enforce --off needs an enrolled actor
    - ✅ verifySignatureBundle: stale timestamp -> SIGNATURE_STALE under enforce
    - ✅ verifySignatureBundle: reused nonce -> NONCE_REUSED even in record mode
    - ✅ verifySignatureBundle: record mode tolerates unknown/revoked/invalid as unsigned
    - ✅ lifecycle: keygen(unlock)→register→enforce→signed→refused→lock→revoke→rotate→verify→company B

### agent-layer.test.js — MCP server, FX/ECB, tool gates, compliance calendar, MCP signed execution (verified rows, enforce refusal, nonces, per-DB registry)

30 passing · 0 failing

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
    - ✅ MCP: resolveMcpFx never stores the fetched ECB rate on a plan-only call (dry-run write regression)
    - ✅ compliance: quarterly deadlines
    - ✅ compliance: jaarrekening deadline is 13 months after the fiscal year end
    - ✅ compliance: calendar shows obligations, statuses flip with filings
    - ✅ compliance: closed books show on the jaarrekening obligation
    - ✅ MCP: initialize + tools/list + read-only calls work end-to-end
    - ✅ MCP: params:null on a call answers cleanly (no -32603 internal error)
    - ✅ MCP: invoices tool derives the overdue status (regression)
    - ✅ MCP: non-object JSON-RPC messages get Invalid Request, server survives
    - ✅ MCP: mutations are plan-only by default; execute books with the actor
    - ✅ MCP: assets_run books DEPRECIATION, not recurring entries (import-collision regression)
    - ✅ MCP: contact_add preserves postal_code and vat_id (regression)
    - ✅ MCP: BUKIO_MCP_READONLY blocks execution
    - ✅ fx resolveRate: a dry-run must not persist the fetched ECB rate
    - ✅ MCP: signed execute call -> audit row verified; audit verify reports ok
    - ✅ MCP: enforce on + missing key -> error response, no mutation (dry-run too)
    - ✅ MCP: repeated signed calls verify (fresh nonces, no replay refusal)
    - ✅ MCP: malformed actor still rejected (INVALID_ACTOR)
    - ✅ MCP: a second company DB uses its own registry/enforce state

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

### authz-cli.test.js — Tier 0.5 authorizations end-to-end: actor authz/roles/can/who-can CLI, the AUTHZ_DENIED gate (dry-run parity, deny-by-default, authz implies enforce), MCP tool gate (no mutation on refusal, read-only unaffected), owner-mediated revoke, full SoD lifecycle

25 passing · 0 failing

    - ✅ authz --on: sets authz on, implies signing enforcement, grants the flipper owner
    - ✅ authz --on --dry-run: nothing is written (no owner, no mode change)
    - ✅ authz: exactly one of --on/--off is required (INVALID_AUTHZ)
    - ✅ authz --off: the owner turns authz off; signing enforcement STAYS on
    - ✅ authz --off: a non-owner is refused AUTHZ_DENIED under authz
    - ✅ roles grant/revoke: audit rows + SoD warning on a conflicting grant
    - ✅ roles revoke: ROLE_NOT_GRANTED on absent role; LAST_OWNER guards the last owner
    - ✅ roles: invalid role and invalid grantee are rejected
    - ✅ roles: self-service list for any enrolled actor; --actor <other> is owner only
    - ✅ roles grant/revoke: work for any enrolled actor when authz is OFF (roles are inert data)
    - ✅ actor can: self-service capability check with the ACTUAL mutation
    - ✅ actor can --actor <other>: owner only under authz
    - ✅ who-can: the SoD review lens — owner sees the full matrix
    - ✅ gate: an actor with the right role acts; wrong capability is refused AUTHZ_DENIED before any mutation
    - ✅ gate: B (payments) can create a SEPA batch but not post entries
    - ✅ gate: deny-by-default — a role-less actor can only self-service
    - ✅ gate: dry-run is refused identically (D6 — a plan needs the capability)
    - ✅ gate: reads are gated too — a role-less actor cannot run report trial-balance
    - ✅ gate: entry add --post needs entry.post — the ACTUAL mutation decides
    - ✅ revoke --target: owner kills a compromised key; the target is refused everywhere after
    - ✅ revoke --target: needs the OWNER role REGARDLESS of authz mode (D8)
    - ✅ MCP gate: tool calls map to the same capabilities; refusals mutate nothing
    - ✅ MCP gate: read-only tools are unaffected (not gated) — a role-less actor can still read
    - ✅ MCP gate: vat_book maps to vat.book — a payments actor is refused
    - ✅ lifecycle: owner bootstraps authz, splits bookkeeping/payments, the SoD boundary holds end-to-end

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

### cli.test.js — CLI end-to-end: init, entries, reports, backup/restore

31 passing · 0 failing

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
    - ✅ report balance-sheet/pnl/journal: JSON + CSV + XLSX export
    - ✅ report balance-sheet --as-of is respected
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
    - ✅ actor: help lists the identity subcommands
    - ✅ actor enforce: needs exactly one of --on/--off (INVALID_ENFORCE, JSON contract)
    - ✅ audit verify: clean signed trail -> JSON summary, exit 0
    - ✅ audit verify: tampered row -> exit 1 with per-row status and counts
    - ✅ version: --version and the MCP serverInfo match package.json (drift guard)

### company-simulation.test.js — 

12 passing · 0 failing

    - ✅ stage 1: init (dry-run then real), capital, bank account, company profile
    - ✅ stage 2: contacts (NL/EU customers, NL/EU suppliers) + items catalog
    - ✅ stage 3: sales with discounts, mixed rates, verlegde EU levering, credit note
    - ✅ stage 4: purchases — 21%, EU verlegd (RE), binnenlands verlegd (R)
    - ✅ stage 5: bank import + auto-match — 9 transactions reconcile
    - ✅ stage 6: Q1 OB readout — 1a/1b/2a/3a/3b/4a/4b/5a/5b/5d
    - ✅ stage 7: Q1 vat file + settle — position 2510, whole-euro payment, rounding to 4700
    - ✅ stage 8: P&L, sales by contact, aging, contact statement, month-end check
    - ✅ stage 9: Q2 — sale + purchase, second readout/file/settle cycle
    - ✅ stage 10: payables + SEPA batch — two suppliers in one pain.001
    - ✅ stage 11: year-end close, jaarrekening micro, ICP readout
    - ✅ stage 12: final verification — balanced books, bank, audit, backup

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

### hardening.test.js — 

86 passing · 0 failing

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
    - ✅ CLI: assets register --format json emits JSON without the global --json flag (round 11)
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
    - ✅ report balance-sheet rejects a garbage as-of (it silently read as "forever" before)
    - ✅ report pnl / journal / trial-balance reject a garbage year (no abc-01-01 ranges)
    - ✅ entry list rejects garbage date bounds (--date-to garbage returned ALL entries before)
    - ✅ import opening-balances accepts the documented optional header row (2- and 3-column)
    - ✅ MCP entry_add dry-run validates like execute (garbage date/unbalanced/single-posting rejected, no isError:false plan)
    - ✅ MCP entry_reverse / invoice_credit / invoice_pay dry-runs validate like execute
    - ✅ init validates iban, vat choice and fiscal-year-end (garbage was stored silently)
    - ✅ account add/deactivate/reactivate/import are audited (they mutated silently before)
    - ✅ every emitted error code in src/ is documented in AGENTS.md §7
    - ✅ MCP on a missing database errors NO_DATABASE instead of silently creating an empty company

### invoice-features.test.js — 

38 passing · 0 failing

    - ✅ fractional quantities parse to milli-units
    - ✅ line discounts parse (pct and amount)
    - ✅ item specs parse (id, qty, overrides, discount)
    - ✅ allocateLargestRemainder sums exactly and is deterministic
    - ✅ item add/list/show/update/deactivate with audit
    - ✅ item update: empty string clears vatCode/glAccount instead of keeping the old value (round 11)
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
    - ✅ invoice language: nl default, any i18n table accepted, unknown rejected
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
    - ✅ review fix: PDF reverse-charge label + email language follow the document language (no Dutch fallback)

### jurisdictions.test.js — 

145 passing · 0 failing

    - ✅ getProfile returns the NL profile for NL (any case)
    - ✅ getProfile rejects malformed country input with INVALID_COUNTRY
    - ✅ getProfile throws PROFILE_NOT_FOUND for unknown valid codes
    - ✅ profiles are deep-frozen (static data — no consumer may mutate)
    - ✅ NL profile integrity — tax section matches the legacy VAT module
    - ✅ NL profile integrity — reporting section matches the legacy chart
    - ✅ NL profile integrity — identifiers, compliance, documents, closing
    - ✅ normalizeCountry trims and uppercases
    - ✅ resolveProfile returns the NL profile for a company with country NL
    - ✅ resolveProfile defaults to NL on a pre-021 DB (no country column)
    - ✅ resolveProfile defaults to NL when no company row exists yet
    - ✅ resolveProfile throws for unsupported / unknown company countries (decision §9.1.6)
    - ✅ M3 init: --country IS (valid code, no profile) is rejected with PROFILE_NOT_FOUND
    - ✅ M3 init: --country ZZ (valid code, no profile) is rejected with PROFILE_NOT_FOUND
    - ✅ M3 init: --country nl (lowercase) normalizes to NL and stores profile fields
    - ✅ M3 init: generic --registration-id/--tax-id are stored; no deprecation warning
    - ✅ M3 init: --registration-id/--tax-id set the company identifiers
    - ✅ M3 company update: changing country is rejected with COUNTRY_IMMUTABLE
    - ✅ M3 company update: --country with the SAME value passes the immutability gate
    - ✅ M3 company update: --registration-id updates registration_id
    - ✅ M3 company update: generic --registration-id/--tax-id work without warnings
    - ✅ M4: obReadout resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M4: validateCompliance resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M5: jaarrekening resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M6: compliance status resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M7: invoiceToUbl resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M8: year-end close resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M9: exportXaf resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M9: bank import resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ review-fix: account add --taxonomy-code works
    - ✅ B1: getProfile returns the LU profile (French, PCN 2020 data)
    - ✅ B1: LU is implemented (all thirty-one markets landed)
    - ✅ B1: the LU profile is deep-frozen
    - ✅ B1: init --country LU creates a French LU company with the PCN chart
    - ✅ B1: LU strict dispatch — unregistered formats fail loudly (no NL fallback)
    - ✅ B1: LU UBL invoice emits the RCS scheme 0195 and country LU (never 9944)
    - ✅ B6: LU invoice finalizes end-to-end (compliance rule set registered)
    - ✅ B6: LU supplier requirements — missing RCS / TVA fail with French messages
    - ✅ B6: LU reverse charge requires the customer TVA number (auto-liquidation)
    - ✅ B6: NL invoice compliance is unchanged (byte-identical, nl-12-vereisten)
    - ✅ B2: LU financial statements report the LSC abridged layout
    - ✅ DE: UBL reverse-charge line percent is profile-driven, not NL 21.00 (review fix)
    - ✅ BE: vat book auto VAT legs land on the profile ledger, not NL 2500/1500 (review fix)
    - ✅ FR: vat book accepts dotted VAT codes (@5.5) and posts to 44571 (review fix)
    - ✅ BE: vat file/settle resolve the profile defaults via the CLI (review fix)
    - ✅ B2: LU P&L — mixed leftover (custom expense + custom income) reconciles (review fix)
    - ✅ B2: LU P&L — 73x subventions on line 4 and custom expenses subtract (review fix)
    - ✅ B2: cross-border buyer EndpointID uses the BUYER country scheme (review fix)
    - ✅ B2: LU financial statements reject the NL model (INVALID_MODEL)
    - ✅ B2: NL financial statements keep the klein default (byte-identical)
    - ✅ B5: LU compliance calendar — TVA on the 15th + annual accounts in 7 months
    - ✅ B5: LU TVA filings mark through the registry and flip the status
    - ✅ B3: LU export xaf produces the FAIA 2.01 reduced-B audit file
    - ✅ B3: FAIA omits the TaxTable for a TVA-less company (review fix)
    - ✅ B3: NL XAF export is unchanged (byte-identical, xaf-auditfile-4.0)
    - ✅ GB: getProfile returns the GB profile (GBP, en-GB, UK conventions)
    - ✅ GB: init --country GB creates a GBP company with the UK chart
    - ✅ GB: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ GB: compliance calendar — annual accounts in 9 months, CT600 in 12
    - ✅ FR: getProfile returns the FR profile (EUR, fr, PCG data)
    - ✅ FR: init --country FR creates a French company with the PCG chart
    - ✅ FR: dotted VAT codes (5.5/2.1) parse in the invoice line spec (review fix)
    - ✅ FR: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ US: getProfile returns the US profile (USD, en-US, no federal VAT)
    - ✅ US: init --country US creates a USD company with the US chart
    - ✅ US: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ US: compliance calendar — 1120 on 15 Apr + 941 quarterly (month-end)
    - ✅ BE: getProfile returns the BE profile (EUR, nl-BE, PCN-BE data)
    - ✅ BE: init --country BE creates a Belgian company with the PCMN chart
    - ✅ BE: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ BE: compliance calendar — VAT on the 20th + annual accounts in 7 months
    - ✅ DE: bank add defaults to the profile bank account (1200), not NL 1100 (review fix)
    - ✅ NL: bank add still defaults to 1100 (byte-identity)
    - ✅ DE: getProfile returns the DE profile (EUR, de-DE, SKR 03 data)
    - ✅ DE: init --country DE creates a German company with the SKR 03 chart
    - ✅ DE: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ DE: compliance calendar — UStVA 10th + annual VAT 31 Jul + accounts 12 mo
    - ✅ DK: getProfile returns the DK profile (DKK, da-DK, 25% VAT only)
    - ✅ DK: init --country DK creates a Danish company with the kontoplan
    - ✅ DK: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ DK: compliance calendar — quarterly VAT 1st of 3rd month + accounts 5 months
    - ✅ FI: getProfile returns the FI profile (EUR, fi-FI, 25.5% VAT)
    - ✅ FI: init --country FI creates a Finnish company with the model chart
    - ✅ FI: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ FI: compliance calendar — quarterly VAT 12th of 2nd month + accounts in 8 months
    - ✅ NO: getProfile returns the NO profile (NOK, nb-NO, NS 4102)
    - ✅ NO: init --country NO creates a Norwegian company with the NS 4102 chart
    - ✅ NO: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ NO: compliance calendar — bi-monthly VAT (6/yr) + accounts by 31 July
    - ✅ SE: getProfile returns the SE profile (SEK, sv-SE, BAS 2023)
    - ✅ SE: init --country SE creates a Swedish company with the BAS chart
    - ✅ SE: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ SE: compliance calendar — quarterly VAT 12th of 2nd month (Aug 17th) + accounts 7 months
    - ✅ AT: getProfile returns the AT profile (EUR, de-AT, EKR data)
    - ✅ AT: init --country AT creates an Austrian company with the EKR chart
    - ✅ AT: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ AT: compliance calendar — UVA 15th of second following month + annual VAT 30 Jun
    - ✅ IE: getProfile returns the IE profile (EUR, en, UK-style chart)
    - ✅ IE: init --country IE creates an Irish company with the UK-style chart
    - ✅ IE: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ IE: compliance calendar — VAT3 bi-monthly 23rd + annual accounts/CT1 9 months
    - ✅ IT: getProfile returns the IT profile (EUR, it, convention chart)
    - ✅ IT: init --country IT creates an Italian company with the convention chart
    - ✅ IT: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ IT: compliance calendar — liquidazione 16th of 2nd month + Dichiarazione 30 Apr + bilancio
    - ✅ ES: getProfile returns the ES profile (EUR, es, PGC chart)
    - ✅ ES: init --country ES creates a Spanish company with the PGC chart
    - ✅ ES: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ ES: compliance calendar — Modelo 303 quarterly + 390 + 200 + cuentas anuales
    - ✅ PT: getProfile returns the PT profile (EUR, pt, SNC chart)
    - ✅ PT: init --country PT creates a Portuguese company with the SNC chart
    - ✅ PT: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ PT: compliance calendar — Declaração Periódica 20th of 2nd month + IRC + IES
    - ✅ EU baseline: a DE company finalizes invoices end-to-end (art. 226 rule + de document language)
    - ✅ BG: getProfile returns the BG profile (EUR, bg, NSS chart)
    - ✅ BG: init --country BG creates a Bulgarian company with the NSS chart
    - ✅ BG: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ HR: getProfile returns the HR profile (EUR, hr, Računski plan)
    - ✅ HR: init --country HR creates a Croatian company with the Računski plan chart
    - ✅ SI: getProfile returns the SI profile (EUR, si, SRS 30 kontni načrt)
    - ✅ SI: init --country SI creates a Slovenian company (language defaults to sl)
    - ✅ EE: getProfile returns the EE profile (EUR, ee, RMP convention chart)
    - ✅ EE: init --country EE creates an Estonian company (language defaults to et)
    - ✅ LV: getProfile returns the LV profile (EUR, lv, standard kontu plāns)
    - ✅ LV: init --country LV creates a Latvian company (language defaults to lv)
    - ✅ LT: getProfile returns the LT profile (EUR, lt, Įmonių sąskaitų planas)
    - ✅ LT: init --country LT creates a Lithuanian company (language defaults to lt)
    - ✅ MT: getProfile returns the MT profile (EUR, mt, convention chart)
    - ✅ MT: init --country MT creates a Maltese company (language defaults to mt)
    - ✅ CY: getProfile returns the CY profile (EUR, cy, convention chart)
    - ✅ CY: init --country CY creates a Cypriot company (language defaults to cy)
    - ✅ CZ: getProfile returns the CZ profile (CZK, cz, směrná účtová osnova)
    - ✅ CZ: init --country CZ creates a Czech company (language defaults to cs)
    - ✅ SK: getProfile returns the SK profile (EUR, sk, směrná účtová osnova)
    - ✅ SK: init --country SK creates a Slovak company (language defaults to sk)
    - ✅ GR: getProfile returns the GR profile (EUR, gr, ΕΓΛΣ chart; EL prefix)
    - ✅ GR: init --country GR creates a Greek company (language defaults to el)
    - ✅ PL: getProfile returns the PL profile (PLN, pl, Rozporządzenie MF chart)
    - ✅ PL: init --country PL creates a Polish company (language defaults to pl)
    - ✅ HU: getProfile returns the HU profile (HUF, hu, Szt. chart)
    - ✅ HU: init --country HU creates a Hungarian company (language defaults to hu)
    - ✅ RO: getProfile returns the RO profile (RON, ro, Planul de conturi)
    - ✅ RO: init --country RO creates a Romanian company (language defaults to ro)
    - ✅ XK: getProfile returns the XK profile (EUR, sq, SKRFI convention chart)
    - ✅ XK: init --country XK creates a Kosovar company (language defaults to sq)

### remote.test.js — 

20 passing · 0 failing

    - ✅ server token: mints a single-use, actor-bound token (hashed at rest)
    - ✅ remote register: enrols a client-only key (private key never leaves the client)
    - ✅ remote register: a used token is refused (TOKEN_USED)
    - ✅ remote register: an unknown / mismatched token is refused
    - ✅ remote register: --token is required with --server (TOKEN_REQUIRED)
    - ✅ remote register: an expired token is refused (TOKEN_EXPIRED)
    - ✅ remote read: trial balance matches the local view (same device OK)
    - ✅ remote mutation: posts an entry, the audit row carries the REAL signature
    - ✅ remote mutation: dry-run parity (plan, no side effect)
    - ✅ remote human output: byte-identical to local human output
    - ✅ replay: the SAME envelope twice is refused (NONCE_REUSED)
    - ✅ tamper: changing the signed argv breaks the signature (SIGNATURE_INVALID)
    - ✅ enforcement: an unsigned envelope is refused under enforce (SIGNATURE_REQUIRED)
    - ✅ authz: a readonly actor is refused a mutation (AUTHZ_DENIED)
    - ✅ local-only commands refuse under --server (LOCAL_ONLY)
    - ✅ health endpoint reports ok
    - ✅ unknown route is 404
    - ✅ unreachable server: clean REMOTE_UNREACHABLE error
    - ✅ server token rejects a bad --ttl-hours value
    - ✅ envelope can carry the --db of the CLIENT but the server DB is authoritative

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

---
_Regenerated automatically on every `npm test`._
