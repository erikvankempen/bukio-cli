# bukio-cli — test report

**Latest run:** 2026-08-15 07:33:04 UTC — **✅ 853 passing · 0 failing (853 tests)**
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

### actor-registry.test.js — per-company actor key registry: enrol/revoke (history kept), rotation, enforce flag, per-DB independence + Tier 0.5 role registry (grant/revoke/getRoles, authz flag, last-owner guard)

18 passing · 0 failing

    - ✅ enrol: new actor writes a registry row with keyid, public key and timestamp
    - ✅ enrol: duplicate enrol while an active key exists fails ALREADY_ENROLLED
    - ✅ enrol: invalid actor or missing key material is rejected
    - ✅ revoke: marks the row with reason and keeps it (history retained)
    - ✅ revoke: requires a reason; unknown or already-revoked actors are rejected
    - ✅ canAct: true for enrolled, false for unknown and revoked actors
    - ✅ rotation: re-enrol after revocation adds a fresh active key; the old key row is retained
    - ✅ enforce: flag defaults to off, toggles per DB, and is independent
    - ✅ registry is per company DB: same actor, independent enrolments
    - ✅ registry persists to disk and survives reopen (file-backed DB)
    - ✅ authz: flag defaults to off, toggles per DB, and is independent
    - ✅ grantRole: writes a row with granted_by/granted_at; idempotent on repeat
    - ✅ grantRole: rejects invalid actors and invalid roles
    - ✅ revokeRole: removes the row; revoking a role not held fails ROLE_NOT_GRANTED
    - ✅ revokeRole: the LAST owner can never be revoked (flipper-bootstrap guarantee)
    - ✅ roles: per-company independence — grants in one DB do not leak to another
    - ✅ listRoleGrants: every grant row with grantor + timestamp
    - ✅ roles: authz flag and role grants persist to disk and survive reopen

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

### audit.test.js — audit log record/list, append-only trigger, migration 018/019, signature columns + verifyTrail classification matrix

19 passing · 0 failing

    - ✅ record + list with filters
    - ✅ audit log is append-only: UPDATE and DELETE are blocked
    - ✅ args null is stored and read back as null
    - ✅ migration 018: fresh DB gains the six signature columns + actor_keys + settings
    - ✅ migration 019: actor_keys gains a composite (actor, keyid) primary key
    - ✅ migration 019: a v18 DB with single-row actor_keys upgrades without data loss
    - ✅ migration 018: existing DB keeps legacy rows with sig_status = unsigned
    - ✅ migration 018: re-running migrate on the current version is a no-op
    - ✅ record: signature fields are stored and read back; plain records default to unsigned
    - ✅ verifyTrail: clean signed trail -> all ok with matching summary counts
    - ✅ verifyTrail: negative or non-integer limit -> INVALID_LIMIT (parity with audit list)
    - ✅ verifyTrail: tampered args_json -> tampered (digest no longer matches)
    - ✅ verifyTrail: corrupted signature -> invalid-signature
    - ✅ verifyTrail: keyid not in the registry -> unknown-key
    - ✅ verifyTrail: signature from a since-revoked key -> revoked (valid at the time)
    - ✅ verifyTrail: rotation keeps old rows verifiable (old revoked, new ok)
    - ✅ verifyTrail: --limit checks only the newest N rows
    - ✅ verifyTrail: works on a copied DB file with no external files (self-contained)
    - ✅ verifyTrail: --since filters rows by timestamp

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

### authz.test.js — Tier 0.5 capability map: command→capability coverage (§3 + full CLI + MCP tools), canAct matrix, SoD warnings, exemption set, authz gate (unit)

18 passing · 0 failing

    - ✅ capabilityOf: entry add resolves by the ACTUAL mutation (--post)
    - ✅ capabilityOf: every documented §3 command maps to exactly one capability
    - ✅ capabilityOf: every real CLI command path maps or is authz-exempt
    - ✅ capabilityOf: every MCP mutating tool maps to exactly one capability
    - ✅ capabilityOf: unmapped commands return null (fail closed)
    - ✅ canAct: deny-by-default — no roles grants nothing
    - ✅ canAct: owner passes EVERYTHING; roles grant only their capabilities
    - ✅ canAct: fail closed — a null capability is never granted
    - ✅ role definitions are consistent: every listed capability is a real capability
    - ✅ sodWarnings: the documented conflict pairs warn; clean sets stay quiet
    - ✅ sodWarnings: every documented pair is real (map stays in sync)
    - ✅ isAuthzExempt: self-service + bootstrap commands are exempt; owner actions are not
    - ✅ checkAuthz: authz off (default) → no refusals
    - ✅ checkAuthz: unmapped command denies under authz (fail closed)
    - ✅ checkAuthz: AUTHZ_DENIED message names the actor, missing capability and roles
    - ✅ checkAuthz: dry-run is refused identically (capability required for the plan)
    - ✅ checkAuthz: owner-mediated revoke needs the owner role REGARDLESS of authz mode (D8)
    - ✅ checkAuthz: no DB → no authz state → never refuses

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

23 passing · 0 failing

    - ✅ parseCamt053: CRDT positive, DBIT negative, counterparty + description
    - ✅ parseCamt053: rejects non-CAMT input
    - ✅ parseCamt053: empty party-name element falls through to the other party
    - ✅ parseBankAmount: degenerate inputs (., ,) return null, never NaN
    - ✅ parseBankAmount: Dutch and international formats
    - ✅ parseBankCsv: Rabo-style export with Af/Bij sign
    - ✅ parseBankCsv: missing required columns rejected
    - ✅ importTransactions: idempotent via hash (duplicates skipped)
    - ✅ previewImport: dry-run counts without writing
    - ✅ getOrCreateBankAccount: validates IBAN and links to ledger account
    - ✅ getOrCreateBankAccount: dashed IBAN normalizes to the stored form (no duplicate account)
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
    - ✅ parseBankCsv: Dutch DD-MM-YYYY and compact YYYYMMDD dates normalize to ISO
    - ✅ parseBankCsv: an unparseable date is skipped and reported, never silently dropped

### canonical.test.js — canonical command digest: stable sorted-key JSON, sha256, excludes identity/output flags, includes --dry-run

7 passing · 0 failing

    - ✅ canonical: same input -> same digest regardless of key order
    - ✅ canonical: different args -> different digest
    - ✅ canonical: different actor, cmd, ts or nonce -> different digest
    - ✅ canonical: excludes --actor, --sign-key and --json from the signed args
    - ✅ canonical: includes --dry-run in the signed args
    - ✅ canonical: nested args (postings, lines) are stable and order-insensitive
    - ✅ canonical: canonicalJson is deterministic pretty-printed JSON with sorted keys

### cli.test.js — CLI end-to-end: init, entries, reports, backup/restore

32 passing · 0 failing

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
    - ✅ report balans stays available as a deprecated alias
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

37 passing · 0 failing

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
    - ✅ vat: private use (P) VAT is ALWAYS owed (credit 2500) regardless of the posting sign
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
    - ✅ ensureDb(mustExist:false) returns null and never creates the database file

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

### import-invoice.test.js — inbound UBL (EN 16931/Peppol) invoice import into payables: idempotent, VAT reported not booked

14 passing · 0 failing

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
    - ✅ importUblInvoice: missing cbc:InvoiceTypeCode (EN 16931 BT-3) is rejected
    - ✅ importUblInvoice: missing cbc:DocumentCurrencyCode (EN 16931 BT-5) is rejected
    - ✅ importUblInvoice: a malformed PayableAmount is collected with other errors, not thrown mid-parse (round 11)

### import.test.js — opening balances, journal CSV, XAF (both layouts), contacts — whole-file validation, RGS inference

39 passing · 0 failing

    - ✅ parseImportAmount: international, Dutch comma, thousands-dot forms
    - ✅ opening-balances: imports ONE posted Beginbalans entry (source import)
    - ✅ opening-balances: Dutch code,debet,credit layout
    - ✅ opening-balances: validation collects ALL errors, writes nothing
    - ✅ opening-balances: re-import is rejected
    - ✅ opening-balances: re-import succeeds after reversing the opening entry (correction path)
    - ✅ opening-balances: dry-run validates and writes nothing
    - ✅ opening-balances: unknown account and zero amount rejected
    - ✅ journal: one posted entry per boekstuk, two postings per line
    - ✅ journal: idempotent re-import skips existing boekstukken
    - ✅ journal: comma-delimited file with a semicolon inside a quoted field parses (delimiter decided once)
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

37 passing · 0 failing

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

29 passing · 0 failing

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
    - ✅ finalize: a number collision inside the transaction is retried, not thrown
    - ✅ markPaid: payment insert + status update are atomic (rollback leaves no payment)
    - ✅ UBL: Peppol BIS 3.0 structure
    - ✅ UBL: seller + buyer EndpointID (BT-34/BT-49) when KVK numbers are present
    - ✅ UBL: credit note uses CreditNote root + type 381 (Peppol BIS 3.0)
    - ✅ UBL: both parties carry cac:PartyLegalEntity/RegistrationName (BT-27/BT-44, 1..1)
    - ✅ UBL: no empty PayeeFinancialAccount when the company has no IBAN (BG-17 cbc:ID 1..1)
    - ✅ UBL: credit note BT-10 buyer reference carries the original klantkenmerk (not the invoice number)
    - ✅ UBL: XML control characters in descriptions are stripped (Peppol-safe)
    - ✅ bank auto-match: incoming payment pays the invoice and posts Bank/Debiteuren
    - ✅ buildInvoicePostings: sales vs credit sign flip
    - ✅ buildInvoicePostings: VAT module off still honors per-line GL accounts (round 11)
    - ✅ invoiceReminders: overdue + due-soon, excludes paid and far-future
    - ✅ invoiceReminders: within-days controls the due-soon window
    - ✅ invoiceReminders: credit notes are not reminder candidates
    - ✅ validateCompliance: VAT-exempt company without btw-id can still invoice
    - ✅ validateCompliance: VAT company without btw-id still fails SUPPLIER_INCOMPLETE
    - ✅ createContact: dashed IBAN is stored in the canonical dash-free form (normalizer parity)

### jurisdictions.test.js — 

78 passing · 0 failing

    - ✅ getProfile returns the NL profile for NL (any case)
    - ✅ getProfile rejects malformed country input with INVALID_COUNTRY
    - ✅ getProfile throws COUNTRY_NOT_SUPPORTED for valid-but-planned countries
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
    - ✅ M3 init: --country LT (valid code, no profile) is rejected with PROFILE_NOT_FOUND
    - ✅ M3 init: --country ZZ (valid code, no profile) is rejected with PROFILE_NOT_FOUND
    - ✅ M3 init: --country nl (lowercase) normalizes to NL and stores profile fields
    - ✅ M3 init: generic --registration-id/--tax-id are stored; no deprecation warning
    - ✅ M3 init: legacy --kvk/--btw-id aliases map to the generic fields and warn
    - ✅ M3 company update: changing country is rejected with COUNTRY_IMMUTABLE
    - ✅ M3 company update: --country with the SAME value passes the immutability gate
    - ✅ M3 company update: --kvk alias warns and updates registration_id
    - ✅ M3 company update: generic --registration-id/--tax-id work without warnings
    - ✅ M4: obReadout resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M4: validateCompliance resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M5: jaarrekening resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M5: deprecated alias `jaarrekening report` still works and warns
    - ✅ M6: compliance status resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M7: invoiceToUbl resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M8: year-end close resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M9: exportXaf resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ M9: bank import resolves the profile (unknown company country -> PROFILE_NOT_FOUND)
    - ✅ review-fix: account add --taxonomy-code works; --rgs-code alias maps and warns
    - ✅ B1: getProfile returns the LU profile (French, PCN 2020 data)
    - ✅ B1: LU is implemented — PLANNED is FI/NO/SE
    - ✅ B1: the LU profile is deep-frozen
    - ✅ B1: init --country LU creates a French LU company with the PCN chart
    - ✅ B1: LU strict dispatch — unregistered formats fail loudly (no NL fallback)
    - ✅ B1: LU UBL invoice emits the RCS scheme 0195 and country LU (never 9944)
    - ✅ B6: LU invoice finalizes end-to-end (compliance rule set registered)
    - ✅ B6: LU supplier requirements — missing RCS / TVA fail with French messages
    - ✅ B6: LU reverse charge requires the customer TVA number (auto-liquidation)
    - ✅ B6: NL invoice compliance is unchanged (byte-identical, nl-12-vereisten)
    - ✅ B2: LU financial statements report the LSC abridged layout
    - ✅ B2: LU financial statements reject the NL model (INVALID_MODEL)
    - ✅ B2: NL financial statements keep the klein default (byte-identical)
    - ✅ B5: LU compliance calendar — TVA on the 15th + annual accounts in 7 months
    - ✅ B5: LU TVA filings mark through the registry and flip the status
    - ✅ B3: LU export xaf produces the FAIA 2.01 reduced-B audit file
    - ✅ B3: NL XAF export is unchanged (byte-identical, xaf-auditfile-4.0)
    - ✅ GB: getProfile returns the GB profile (GBP, en-GB, UK conventions)
    - ✅ GB: PLANNED is FI/NO/SE
    - ✅ GB: init --country GB creates a GBP company with the UK chart
    - ✅ GB: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ GB: compliance calendar — annual accounts in 9 months, CT600 in 12
    - ✅ FR: getProfile returns the FR profile (EUR, fr, PCG data)
    - ✅ FR: PLANNED is FI/NO/SE
    - ✅ FR: init --country FR creates a French company with the PCG chart
    - ✅ FR: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ US: getProfile returns the US profile (USD, en-US, no federal VAT)
    - ✅ US: PLANNED is FI/NO/SE
    - ✅ US: init --country US creates a USD company with the US chart
    - ✅ US: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ US: compliance calendar — 1120 on 15 Apr + 941 quarterly (month-end)
    - ✅ BE: getProfile returns the BE profile (EUR, nl-BE, PCN-BE data)
    - ✅ BE: PLANNED is FI/NO/SE
    - ✅ BE: init --country BE creates a Belgian company with the PCMN chart
    - ✅ BE: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ BE: compliance calendar — VAT on the 20th + annual accounts in 7 months
    - ✅ DE: getProfile returns the DE profile (EUR, de-DE, SKR 03 data)
    - ✅ DE: PLANNED is FI/NO/SE
    - ✅ DE: init --country DE creates a German company with the SKR 03 chart
    - ✅ DE: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ DE: compliance calendar — UStVA 10th + annual VAT 31 Jul + accounts 12 mo
    - ✅ DK: getProfile returns the DK profile (DKK, da-DK, 25% VAT only)
    - ✅ DK: PLANNED is FI/NO/SE
    - ✅ DK: init --country DK creates a Danish company with the kontoplan
    - ✅ DK: strict dispatch — unregistered formats fail loudly (no fallback)
    - ✅ DK: compliance calendar — quarterly VAT 1st of 3rd month + accounts 5 months

### migration-021.test.js — 

2 passing · 0 failing

    - ✅ migrations 021+022 upgrade a 020 DB: new columns, CHECK removals, renames, backfill
    - ✅ migration 021 keeps company data lossless across the rebuild

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

24 passing · 0 failing

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
    - ✅ parseBatchCsv: comma-delimited rows keep the comma delimiter even when a field contains a semicolon
    - ✅ addPayable: the same (contact, invoice_ref) twice is rejected while unpaid (double-payment guard)
    - ✅ createPaymentBatch: direct-debit lines require a SEPA mandate (pain.008 MndtId)

### recurring-invoice.test.js — subscription invoice templates

11 passing · 0 failing

    - ✅ invoice template: generates draft invoices on schedule (never auto-finalizes)
    - ✅ invoice template: generated drafts finalize normally (compliance + number)
    - ✅ invoice template: guards
    - ✅ invoice template: entry templates keep working alongside
    - ✅ invoice template: dry-run shows the invoice plan, writes nothing
    - ✅ invoice template: runs limit completes the template
    - ✅ peppol send: posts the UBL to the provider (mock server)
    - ✅ peppol send: not configured / provider error / dry-run
    - ✅ peppol send: buyer without a KVK number is rejected up front (BT-49)
    - ✅ peppol send: invoice without a buyer reference is rejected up front (BT-10)
    - ✅ UBL: BuyerReference (BT-10) emitted when the invoice has a reference

### recurring.test.js — recurring entries engine: schedules, depreciation, accruals

22 passing · 0 failing

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
    - ✅ vat-aware template: object postings mixed into a tagged list are kept, not dropped
    - ✅ previewDue: read-only plan matches runDue
    - ✅ listTemplates: status filter
    - ✅ generated entries are immutable + trial balance stays balanced

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

### reports-v014.test.js — aging buckets, contact statements, sales analytics (by contact/item)

13 passing · 0 failing

    - ✅ aging debtors: buckets, totals, paid excluded, contacts sorted by total
    - ✅ aging debtors: invoices issued AFTER the as-of date are excluded, item totals netted by credits
    - ✅ aging debtors: finalized credit notes reduce the outstanding, drafts do not
    - ✅ aging creditors: buckets + in_batch shown separately
    - ✅ aging creditors: payables dated AFTER the as-of date are excluded
    - ✅ aging validation: bad as-of and kind rejected
    - ✅ contact statement: running balance ends at outstanding; supplier side negative
    - ✅ contact statement: credit notes reduce the balance (regression)
    - ✅ contact statement: payments after the as-of date are excluded (as-of leak regression)
    - ✅ sales by contact: net/vat/gross from the totals engine; credit notes excluded
    - ✅ sales by item: catalog items group by item_id, ad-hoc lines by description
    - ✅ cli: report aging + sales + contact statement e2e with csv export
    - ✅ mcp: report_aging and report_sales expose the same shapes

### reports.test.js — balance sheet, P&L, journal

9 passing · 0 failing

    - ✅ balans: assets = liabilities + equity + result
    - ✅ balans: before any income/expense, result is zero
    - ✅ balans: empty books balance at zero
    - ✅ balans: drafts excluded, reversal nets out
    - ✅ pnl: revenue, costs and result
    - ✅ pnl: empty period gives zero result and no sections
    - ✅ pnl: legacy chart without RGS codes still splits revenue/costs by type
    - ✅ pnl: catch-all section for accounts with unknown taxonomy_code
    - ✅ journal: one row per posting, ordered by date

### review-round3.test.js — 

4 passing · 0 failing

    - ✅ recurring pause --dry-run and resume --dry-run render a plan (no fmtTemplate crash)
    - ✅ audit --format json prints JSON even without the global --json flag
    - ✅ bank match post --dry-run rejects an already-matched transaction and a missing account
    - ✅ vat book --dry-run rejects unbalanced postings (parity with entry add)

### sign.test.js — ed25519 sign/verify/keyid module: keygen (plain + passphrase-encrypted PKCS8), roundtrip, tamper/wrong-key rejection

13 passing · 0 failing

    - ✅ sign/verify: roundtrip with a plain key
    - ✅ sign/verify: works with Buffer data too
    - ✅ sign/verify: wrong key fails
    - ✅ sign/verify: tampered message fails
    - ✅ sign/verify: malformed signature or key does not throw, returns false
    - ✅ keyid: stable 32-hex fingerprint of the public key
    - ✅ keygen: writes SPKI public and PKCS8 private PEM
    - ✅ keygen: passphrase-encrypted key refuses to sign without the passphrase
    - ✅ keygen: passphrase-encrypted key signs with the right passphrase and verifies
    - ✅ keyid: fingerprint is identical for plain and passphrase keys sharing a public key
    - ✅ publicKeyFromPrivate: plain key derives its own public key (same keyid)
    - ✅ publicKeyFromPrivate: encrypted key needs the passphrase, wrong one throws
    - ✅ decryptPrivateKey: returns a plain PKCS8 PEM usable for signing; wrong passphrase throws

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

21 passing · 0 failing

    - ✅ year-end close: posts closing + appropriation, balanced, source closing
    - ✅ year-end close: reversing the closing entries re-opens the year (documented undo)
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
    - ✅ jaarrekening PDF: esc() escapes double quotes (attribute-injection regression)
    - ✅ OB readout: R purchase -> 3a/4a, RE purchase -> 3b/4b, RE sale -> 2a
    - ✅ OB readout: verlegde EU sale (RE invoice) reports 2a
    - ✅ ICP readout: EU customers with RE lines, totals per customer
    - ✅ ICP readout: missing customer vat-id fails loudly
    - ✅ ICP readout: no RE lines -> empty listing

---
_Regenerated automatically on every `npm test`._
