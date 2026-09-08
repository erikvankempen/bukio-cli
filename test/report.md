# bukio-cli — test report

**Latest run:** 2026-09-08 13:09:35 UTC — **❌ 25 passing · 72 failing (97 tests)**
**Command:** `npm test` (per-file `node --test --test-reporter=tap`)

## All tests

### accounts.test.js — chart of accounts CRUD + CSV chart import

0 passing · 1 failing

    - ❌ test/accounts.test.js

### actor-registry.test.js — per-company actor key registry: enrol/revoke (history kept), rotation, enforce flag, per-DB independence + Tier 0.5 role registry (grant/revoke/getRoles, authz flag, last-owner guard)

0 passing · 1 failing

    - ❌ test/actor-registry.test.js

### actor.test.js — named-actor enforcement, actor identity CLI + sign-and-verify gate (record/enforce modes, stale/replay/registry refusals) + full Tier 0 lifecycle (enrol→enforce→lock→revoke→rotate→verify→company B)

0 passing · 1 failing

    - ❌ test/actor.test.js

### agent-layer.test.js — MCP server, FX/ECB, tool gates, compliance calendar, MCP signed execution (verified rows, enforce refusal, nonces, per-DB registry)

0 passing · 1 failing

    - ❌ test/agent-layer.test.js

### assets.test.js — fixed assets: schemes, mid-life adoption, runs, disposal, activastaat

0 passing · 1 failing

    - ❌ test/assets.test.js

### attachments.test.js — in-DB/file document attachments: add/list/show/remove, 25 MB cap, dedupe, metadata-only lists, audit

0 passing · 1 failing

    - ❌ test/attachments.test.js

### audit.test.js — audit log record/list, append-only trigger, migration 018/019, signature columns + verifyTrail classification matrix

0 passing · 1 failing

    - ❌ test/audit.test.js

### authz-cli.test.js — Tier 0.5 authorizations end-to-end: actor authz/roles/can/who-can CLI, the AUTHZ_DENIED gate (dry-run parity, deny-by-default, authz implies enforce), MCP tool gate (no mutation on refusal, read-only unaffected), owner-mediated revoke, full SoD lifecycle

0 passing · 1 failing

    - ❌ test/authz-cli.test.js

### authz.test.js — Tier 0.5 capability map: command→capability coverage (§3 + full CLI + MCP tools), canAct matrix, SoD warnings, exemption set, authz gate (unit)

0 passing · 1 failing

    - ❌ test/authz.test.js

### backup.test.js — encrypted backups (AES-256-GCM), keep-N rotation, tamper detection, audited restore

0 passing · 1 failing

    - ❌ test/backup.test.js

### bank.test.js — CAMT.053/CSV import, idempotency, matching/reconciliation

0 passing · 1 failing

    - ❌ test/bank.test.js

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

0 passing · 1 failing

    - ❌ test/cli.test.js

### company-simulation.test.js — 

0 passing · 12 failing

    - ❌ stage 1: init (dry-run then real), capital, bank account, company profile
    - ❌ stage 2: contacts (NL/EU customers, NL/EU suppliers) + items catalog
    - ❌ stage 3: sales with discounts, mixed rates, verlegde EU levering, credit note
    - ❌ stage 4: purchases — 21%, EU verlegd (RE), binnenlands verlegd (R)
    - ❌ stage 5: bank import + auto-match — 9 transactions reconcile
    - ❌ stage 6: Q1 OB readout — 1a/1b/2a/3a/3b/4a/4b/5a/5b/5d
    - ❌ stage 7: Q1 vat file + settle — position 2510, whole-euro payment, rounding to 4700
    - ❌ stage 8: P&L, sales by contact, aging, contact statement, month-end check
    - ❌ stage 9: Q2 — sale + purchase, second readout/file/settle cycle
    - ❌ stage 10: payables + SEPA batch — two suppliers in one pain.001
    - ❌ stage 11: year-end close, jaarrekening micro, ICP readout
    - ❌ stage 12: final verification — balanced books, bank, audit, backup

### company.test.js — company show/update

0 passing · 1 failing

    - ❌ test/company.test.js

### cost-centers.test.js — cost-center registry, @CC posting spec, entry/reversal carry CC, cost-center analysis report

0 passing · 1 failing

    - ❌ test/cost-centers.test.js

### direct-debit.test.js — SEPA direct debit: mandate register, pain.008.001.02 export, FRST/RCUR, CORE/B2B split

0 passing · 1 failing

    - ❌ test/direct-debit.test.js

### edge-cases.test.js — rounding, boundaries, idempotency, lifecycle violations, dry-run hygiene

0 passing · 1 failing

    - ❌ test/edge-cases.test.js

### entries.test.js — journal entries: add/post/reverse, immutability

0 passing · 1 failing

    - ❌ test/entries.test.js

### export.test.js — export xaf (Auditfile 4.0, round-trips through the importer) + audit csv/xlsx

0 passing · 1 failing

    - ❌ test/export.test.js

### fiscal-year.test.js — 

0 passing · 1 failing

    - ❌ test/fiscal-year.test.js

### hardening.test.js — 

0 passing · 1 failing

    - ❌ test/hardening.test.js

### i18n.test.js — 

0 passing · 1 failing

    - ❌ test/i18n.test.js

### import-invoice.test.js — inbound UBL (EN 16931/Peppol) invoice import into payables: idempotent, VAT reported not booked

0 passing · 1 failing

    - ❌ test/import-invoice.test.js

### import.test.js — opening balances, journal CSV, XAF (both layouts), contacts — whole-file validation, RGS inference

0 passing · 1 failing

    - ❌ test/import.test.js

### invoice-features.test.js — 

0 passing · 1 failing

    - ❌ test/invoice-features.test.js

### invoice.test.js — invoicing: lifecycle, 12-vereisten, credit notes, payments, reminders

0 passing · 1 failing

    - ❌ test/invoice.test.js

### jurisdictions.test.js — 

0 passing · 1 failing

    - ❌ test/jurisdictions.test.js

### migration-021.test.js — 

0 passing · 1 failing

    - ❌ test/migration-021.test.js

### money.test.js — integer-cents money helpers

5 passing · 0 failing

    - ✅ parseAmount: valid inputs
    - ✅ parseAmount: rejects invalid inputs
    - ✅ parseAmount: rejects more than 2 decimals
    - ✅ formatAmount: round-trips with parseAmount
    - ✅ formatAmount: formatting

### month-end.test.js — month-end close check

0 passing · 1 failing

    - ❌ test/month-end.test.js

### payments.test.js — SEPA payment batches: payables, pain.001 export

0 passing · 1 failing

    - ❌ test/payments.test.js

### recurring-invoice.test.js — subscription invoice templates

0 passing · 1 failing

    - ❌ test/recurring-invoice.test.js

### recurring.test.js — recurring entries engine: schedules, depreciation, accruals

0 passing · 1 failing

    - ❌ test/recurring.test.js

### remote.test.js — 

0 passing · 20 failing

    - ❌ server token: mints a single-use, actor-bound token (hashed at rest)
    - ❌ remote register: enrols a client-only key (private key never leaves the client)
    - ❌ remote register: a used token is refused (TOKEN_USED)
    - ❌ remote register: an unknown / mismatched token is refused
    - ❌ remote register: --token is required with --server (TOKEN_REQUIRED)
    - ❌ remote register: an expired token is refused (TOKEN_EXPIRED)
    - ❌ remote read: trial balance matches the local view (same device OK)
    - ❌ remote mutation: posts an entry, the audit row carries the REAL signature
    - ❌ remote mutation: dry-run parity (plan, no side effect)
    - ❌ remote human output: byte-identical to local human output
    - ❌ replay: the SAME envelope twice is refused (NONCE_REUSED)
    - ❌ tamper: changing the signed argv breaks the signature (SIGNATURE_INVALID)
    - ❌ enforcement: an unsigned envelope is refused under enforce (SIGNATURE_REQUIRED)
    - ❌ authz: a readonly actor is refused a mutation (AUTHZ_DENIED)
    - ❌ local-only commands refuse under --server (LOCAL_ONLY)
    - ❌ health endpoint reports ok
    - ❌ unknown route is 404
    - ❌ unreachable server: clean REMOTE_UNREACHABLE error
    - ❌ server token rejects a bad --ttl-hours value
    - ❌ envelope can carry the --db of the CLIENT but the server DB is authoritative

### reports-v014.test.js — aging buckets, contact statements, sales analytics (by contact/item)

0 passing · 1 failing

    - ❌ test/reports-v014.test.js

### reports.test.js — balance sheet, P&L, journal

0 passing · 1 failing

    - ❌ test/reports.test.js

### review-round3.test.js — 

0 passing · 1 failing

    - ❌ test/review-round3.test.js

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

0 passing · 1 failing

    - ❌ test/smtp.test.js

### trial-balance.test.js — trial balance invariants

0 passing · 1 failing

    - ❌ test/trial-balance.test.js

### update.test.js — 

0 passing · 1 failing

    - ❌ test/update.test.js

### vat-settle.test.js — 

0 passing · 1 failing

    - ❌ test/vat-settle.test.js

### vat.test.js — optional VAT module: codes, vat book, OB readout 1a–5d

0 passing · 1 failing

    - ❌ test/vat.test.js

### year-end.test.js — annual close, jaarrekening micro/klein, ICP

0 passing · 1 failing

    - ❌ test/year-end.test.js

---
_Regenerated automatically on every `npm test`._
