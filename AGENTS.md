# AGENTS.md — bukio-cli Agent Manual

This file is the **agent's manual** for bukio-cli. Read it before driving the tool. It assumes you have shell access to the machine where bukio-cli is installed and that `bukio` is on PATH (or run `node /path/to/bukio-cli/bin/bukio.js`).

---

## 1. House rules (non-negotiable)

1. **Dry-run before you mutate.** Every mutating command accepts `--dry-run`. Run it, read the plan, then run without `--dry-run`.
2. **Identify yourself.** Every command requires a named actor: `--actor '<role>:<name>'` or env `BUKIO_ACTOR` — `agent:bartholomeus` for the agent, `human:erik` for the owner. A bare `human`/`agent` is rejected (`ACTOR_REQUIRED`/`INVALID_ACTOR`); anonymous actors would pollute the audit trail.
3. **Parse `--json`, never scrape text.** Every command prints one JSON document with `--json`. Exit code 0 = success, 1 = failure.
4. **Never touch the SQLite file directly.** No sqlite3 CLI, no raw SQL. The engine + triggers exist to protect the books. If you need a capability that doesn't exist, say so instead of hacking the DB.
5. **Never delete posted entries.** Correct mistakes with `entry reverse` (which posts a contra-entry) and re-book.
6. **Verify after every mutation.** At minimum: `bukio report trial-balance --json` → `data.balanced` must be `true` (when you've posted anything).
7. **Money is integer cents.** `amount_cents` in JSON is the truth. `amount` strings are for humans. No floats, ever.
8. **Archive every source document.** When booking an invoice (or any posted document — purchase invoice, credit note, bank proof), copy the original PDF/image/XML to `~/.bukio/invoices/` next to the live DB before finishing, named `YYYY-MM-DD_<vendor-slug>_<invoice-number>.<ext>` (see 6.17). The books then carry their paper trail.
9. **Every source file carries the license header.** New `.js`/`.mjs` files start with the standard header — project name, copyright Erik van Kempen, `SPDX-License-Identifier: Apache-2.0` — copied verbatim from any existing `src/` file (shebang files keep the shebang on line 1, header after). Never drop, edit, or reorder it.
10. **Signing is required once a company enforces it.** `bukio actor enforce --on` makes a valid actor signature mandatory: unsigned or unverifiable commands are refused *before anything is written*. Never work around an enforced company — not with `--dry-run` (also refused), not with a different `--actor` (must be enrolled), not by disabling the gate. **Only an enrolled actor can disable enforcement** (`actor enforce --off` is refused for unenrolled/keyless actors) — and it is an audited act: ask first. Consequence accepted by design: if *all* enrolled keys are lost, the CLI is locked out on that company (recovery = restore a backup, or the owner edits the DB directly).
11. **Enrol per company — and enrol before enforcing.** An actor's key is enrolled per company DB (`bukio actor register` runs against the *current* `--db`). Register in every company the actor works in; a key valid in one company is not valid in another. **First enrolment is operator-gated**: under `actor enforce on`, registering a brand-new actor is refused (`ACTOR_KEY_UNKNOWN`) — an already-enrolled actor (the operator) must flip `enforce --off` (itself refused for unenrolled actors) → `actor register` → `enforce --on`, each an audited act. Rotation re-enrolment stays possible under enforce (the revoked actor's `actor register` is exempt — it is the only way back in). If a command is refused with `ACTOR_KEY_UNKNOWN`/`ACTOR_KEY_REVOKED`, run `bukio actor verify` and fix enrolment or rotate (`actor keygen --force` + `actor register`).
12. **Human keys are passphrase-protected; use sessions.** `human:` keys are encrypted — run `bukio actor unlock` once per session (default 12 h, `--ttl-hours` to change) or set `BUKIO_SIGNING_PASSPHRASE`; `actor lock` clears the session. Never store a passphrase in a script next to the key; prefer the session for interactive work.
13. **Cron/system actors sign from their key file.** `system:`/`agent:` keys are plain files — a cron job or agent that has the key file signs automatically. Give unattended jobs their own actor (`system:backup`, `system:cron`) with its own enrolled key, so the audit trail shows *which* system acted. `--sign-key <path>` overrides the key file; the resolution order is `--sign-key` → session → `BUKIO_SIGNING_PASSPHRASE` → `<config>/keys/<role>-<name>.key`.

---

## 2. Environment

| What | How |
|------|-----|
| Database path | `--db <path>` or env `BUKIO_DB` (default `~/.bukio/bukio.db`) |
| Actor | `--actor <who>` or env `BUKIO_ACTOR` — **required**, format `'<role>:<name>'` (e.g. `agent:bartholomeus`, `human:erik`) |
| JSON output | `--json` (global flag — works before or after the subcommand) |
| Signing key | `--sign-key <path>`, or the actor's session, or `BUKIO_SIGNING_PASSPHRASE` (human keys), or the actor's key file at `<config>/keys/<role>-<name>.key` — in that order |
| Config dir | env `BUKIO_CONFIG_DIR` (default `~/.bukio`) — keys in `keys/`, sessions in `sessions/`, nonce cache `nonces.json` |

> **Single company per database.** One company = one database file. To work on company B, point `--db` at its file. Never mix companies in one database.

---

## 3. Command quick reference

| Command | Purpose |
|---------|---------|
| `bukio init --name X [--country NL] [--registration-id ..] [--tax-id ..] [--legal-form bv] [--vat on] [--kor] [--dry-run]` | Create the company database + 29-account RGS-mapped chart; jurisdiction defaults from the country profile (NL: kvk 8 digits, btw-id NL...B01). Deprecated aliases: `--kvk` = `--registration-id`, `--btw-id` = `--tax-id`. Fails `ALREADY_INITIALISED` if done. |
| `bukio entry add --date YYYY-MM-DD --desc ".." --postings "CODE:AMT,CODE:AMT" [--post] [--dry-run]` | Create (and optionally post) a balanced journal entry. |
| `bukio entry post --id N [--dry-run]` | Post a draft entry. |
| `bukio entry reverse --id N [--reason ".."] [--dry-run]` | Post a contra-entry that cancels entry N. |
| `bukio entry list [--state draft\|posted] [--limit N]` | List entries (newest first). |
| `bukio entry show --id N` | One entry + postings. |
| `bukio account add/list/show/deactivate/reactivate` | Chart of accounts management. |
| `bukio account import --file chart.csv [--dry-run]` | Import a chart: `code,name,type,normal_balance[,taxonomy_code]` (legacy header `rgs_code` still accepted). |
| `bukio report trial-balance [--year YYYY]` | Per-account totals; `data.balanced` tells you the books reconcile. |
| `bukio report balance-sheet [--as-of YYYY-MM-DD]` (alias: `balans`, deprecated) | Balance sheet; `data.balanced` must be true. |
| `bukio report pnl [--year YYYY]` | P&L: revenue, costs, result. |
| `bukio report journal [--year YYYY]` | Journal export (one row per posting). |
| `bukio report aging [--as-of D] [--kind debtors\|creditors\|both]` | Open items per contact, bucketed by days past due (current/30/60/90+); creditors show `in_batch` separately. |
| `bukio report sales [--year YYYY] [--by contact\|item]` | Sales revenue for a year: per contact (net/vat/gross via the totals engine) or per item (net after per-line discounts; invoice-level discounts not allocated per line). |
| `bukio contact statement --id N [--as-of D]` | Opgave: invoices + payments + payables with a running balance (positive = contact owes you). |
| `bukio report <cmd> --format csv\|xlsx [--out PATH]` | Export any report; xlsx requires `--out`. |
| `bukio bank add --iban IBAN [--name N] [--account-code CODE] [--dry-run]` | Register a bank account (default ledger code 1100). |
| `bukio bank import --file F --iban IBAN [--dry-run]` | Import CAMT.053 XML or bank CSV (idempotent by hash). |
| `bukio bank match auto [--dry-run]` / `suggest` / `link --tx --entry` / `post --tx --account` | Reconcile: auto-match to entries, or post new entries from unmatched transactions. |
| `bukio bank list` / `transactions [--state]` / `ignore --tx` / `unignore --tx` | Account balances, transaction states, ignore/unignore (own transfers). |
| `bukio vat enable` | Enable the VAT module (accounts 1500/2500 + codes). |
| `bukio vat codes` | List the VAT codes (21/9/0/V/R/RE/M/P with rates + types). |
| `bukio vat book --postings "1100:121.00,8000:-100.00@21" [--post]` | Book VAT-aware entries — `@CODE` computes the VAT leg automatically. |
| `bukio vat readout --period 2026-Q2 [--mark-filed]` | OB-aangifte fields 1a–5d for manual filing. Never auto-files. |
| `bukio vat file --period 2026-Q2 [--account 2510] [--dry-run]` | Reclassify the outstanding VAT position (clears 1500/2500) to 'Af te dragen omzetbelasting' (default 2510, auto-created) at filing. Exact cents — the form is filed rounded. If the requested code is taken by another account, the next free numeric code (2511, …) is used and reported; a custom account works via `--account`. |
| `bukio vat settle --tx <id> [--period] [--account 2510] [--difference-account 4700] [--dry-run]` | Book the bank payment that cancels the af-te-dragen balance (tx must be unmatched; outgoing for te betalen, incoming for a refund). `--account` must match the account `vat file` used (default 2510, e.g. 2511 after a collision). The whole-euro filing vs exact-cents liability rounding difference goes to the P&L difference account (default 4700). Atomic: entry + tx reconciliation commit together. |
| `bukio recurring add --postings "CODE:AMT" --frequency monthly --start DATE [--reverse-previous]` | Create a recurring **entry** template. |
| `bukio recurring add --kind invoice --contact N --lines "2x DESC @ PRICE @21" --frequency monthly --start DATE [--due-days]` | Create a **subscription invoice** template — `run` generates draft invoices (never auto-finalizes). `--due-days 0` = due on the invoice date. |
| `bukio recurring run [--as-of DATE] [--template ID] [--dry-run]` | Generate all due entries/invoice drafts (backfills, idempotent). |
| `bukio recurring preview/list/show --id/pause/resume` | Schedule inspection and control. |
| `bukio depreciation add --cost C --life-months M --start DATE` | Depreciation schedule (remainder-adjusted final run). |
| `bukio invoice peppol-send --id N [--endpoint URL] [--dry-run]` | POST the UBL to a Peppol access-point provider (env `BUKIO_PEPPOL_ENDPOINT` + `BUKIO_PEPPOL_TOKEN`). |
| `bukio item add --name N [--description] [--unit h\|day\|month\|unit\|session\|km\|kg\|project] --price P [--vat] [--gl]` | Add a catalog item (price/VAT snapshot onto invoice lines at creation). |
| `bukio item list [--all]` / `show --id` / `update --id [--price] [--unit] [--vat] [--gl] [--deactivate]` | Inspect/update the catalog; deactivation blocks new invoices, existing keep snapshots. |
| `bukio contact add --name N [--address] [--vat-id]` / `list` | Invoice counterparties. |
| `bukio invoice create --contact N --lines "..." \| --items "..." --date D [--discount-pct P \| --discount-amount A] [--language nl\|en]` | Draft invoice. Lines: `[QTYx] DESC @ PRICE [@ VATCODE] [@ -DISCOUNT]` (fractional qty allowed, per-line discount `@-10%`/`@-25.00`). Items: `ID[:QTY][@PRICE][@VATCODE][@-DISCOUNT]` with per-invoice overrides. Total discount applies BEFORE VAT. |
| `bukio company show` / `update --name --registration-id --tax-id --iban --address --postal-code --city [--country read-only] [--dry-run]` | Company record (audited; country immutable after init); supplier gegevens must be complete before finalize (12-vereisten 1-3). |
| `bukio company update --logo FILE` / `--remove-logo` / `company logo --out FILE` | Store/extract the invoice logo (PNG/JPEG/SVG ≤ 1 MB, ≤ 2048×2048 px, stored as a BLOB in the DB — travels with backups). |
| `bukio attach add --invoice N\|--entry N --file F [--store db\|file] [--note] [--dry-run]` / `attach list --invoice N\|--entry N` / `attach show --id N [--out F] [--force]` / `attach remove --id N [--dry-run]` | Store source documents against invoices/entries. Default `--store db` = BLOB in the DB (travels with backups); `--store file` = copy in `<db>-attachments/` with the path stored. Lists are metadata-only. `show --out` refuses to overwrite without `--force`. |
| `bukio invoice create --contact N --lines "2x DESC @ PRICE @21" --date D` | Draft invoice (12-vereisten validated at finalize). |
| `bukio invoice list [--status draft\|sent\|paid\|overdue] [--type sales\|credit]` / `show --id N` | Inspect invoices. |
| `bukio invoice finalize --id N [--dry-run]` | Sequential number + booking entry (Debiteuren/Omzet/btw). |
| `bukio invoice pdf --id N` / `ubl --id N` / `credit --id N` / `pay --id N --date` | PDF (Chromium), Peppol BIS 3.0 XML, credit notes, payments. |
| `bukio invoice email --id N [--to X] [--subject] [--body] [--no-pdf] [--dry-run]` | Email the finalized invoice PDF via SMTP (`BUKIO_SMTP_*` env — host/port/user/pass/from). Delivery is audited; status is `sent` from finalize onward. |
| `bukio invoice peppol-send --id N [--endpoint URL] [--dry-run]` | POST the UBL to a Peppol access-point provider (env `BUKIO_PEPPOL_ENDPOINT` + `BUKIO_PEPPOL_TOKEN`). |
| `bukio year-end status --year YYYY` / `close --year YYYY [--dry-run]` | Annual close: result -> 9900 -> 3000 (source 'closing'; P&L stays visible). |
| `bukio financial-statements report --year YYYY --model micro\|klein [--format json\|pdf\|xlsx]` | Statutory annual accounts (NL: KVK deposit package as PDF; deprecated alias `jaarrekening report`). |
| `bukio icp readout --period 2026-Q3` | ICP listing: EU btw-verlegde supplies per customer (manual filing aid). |
| `bukio fx set --currency USD --date D --rate 1.0875` | FX rate store (upsert, audited). |
| `bukio fx fetch --currency USD [--date D]` | ECB reference rate (free, no key) on/before a date, stored as source=ECB. |
| `bukio fx list` / `show --currency USD [--limit N]` | Rate store inspection. |
| `bukio entry add / vat book --currency USD [--rate R]` | Foreign-currency purchase invoices -> EUR at booking. Rate resolves: --rate -> stored -> ECB auto-fetch (BUKIO_FX_NO_FETCH=1 disables). Postings keep fx_currency/fx_amount_cents. |
| `bukio mcp` | MCP server over stdio (plan-only mutations unless mode=execute; BUKIO_MCP_READONLY=1 blocks them). **Every mutating tool call is signed by its actor** (payload `mcp:<tool_name>`, same digest scheme/nonce cache as the CLI); per-company enforcement applies — under `actor enforce on` unsigned calls are refused with the same error codes as the CLI. Read-only tools are not gated. |
| `bukio server start --listen <host:port> [--serve-db <path>] [--tls-cert C --tls-key K]` | Serve ONE company DB over HTTP(S): clients send signed command envelopes (`--server <url>` or `BUKIO_SERVER`), the server verifies them against the company registry (signature + ±5 min window + nonce replay + authz) and runs them. Byte-identical output to local mode; audit rows carry the REAL signature (`audit verify` works server-side). A bridge like `mcp` — needs no signature itself. Bind to a trusted network (localhost, tailnet) or use `--tls-cert/--tls-key`. |
| `bukio server token <actor> [--ttl-hours N]` | Mint a one-time enrolment token for a remote actor (operator act on the server machine). Redeem on the client with `bukio actor register --server <url> --token <t>` — the private key never leaves the client. |

**Remote transport (how clients reach the daemon):** the envelope signature
is the security layer (identity, integrity, replay) — the transport only
adds privacy and port access control. Three options, in order of preference:
(1) **SSH tunnel** — daemon on `--listen 127.0.0.1:8787` (default, nothing
exposed), client does `ssh -N -L 8787:127.0.0.1:8787 user@server` and points
`--server http://127.0.0.1:8787` at the local end; zero new listening ports,
SSH key-based access, per-device audit attribution intact. (2) **Tailscale /
WireGuard** — daemon on the server's tailnet IP (`--listen 100.x.y.z:8787`),
reachable only by tailnet members (overlay-encrypted, mTLS on Tailscale);
best for agents/cron needing a stable always-on endpoint. (3) **Native TLS**
— `--tls-cert/--tls-key` on a public interface (self-signed OK, pin it
client-side); access control then rests on signatures alone, so run with
`actor enforce on`. **Never** run plain HTTP on a public interface —
signatures prove *who signed* and *nothing altered*, but do not hide command
contents from eavesdroppers. And do not use `ssh user@server "bukio …"` as a
substitute for `--server`: the command would run with the server-side
actor's key, collapsing every remote user into one identity and destroying
per-device audit attribution (the accountability the signing model exists
for). SSH as a *tunnel* gives both.
| `bukio compliance status --year YYYY` / `mark --type ICP\|JAARREKENING --period ...` | Filing deadlines (OB/ICP/jaarrekening) + manual filing registry. |
| `bukio import opening-balances --file F [--date yyyy-mm-dd] [--dry-run]` | Import opening balances as ONE posted Beginbalans entry (CSV `code,amount` or `code,debet,credit`). Sum must be zero. Re-run fails `OPENING_ALREADY_IMPORTED`. |
| `bukio import journal --file F [--create-missing] [--dry-run]` | SnelStart/Exact-style journal CSV (aliases, `;` delimiter, Dutch amounts). One posted entry per boekstuknummer; idempotent. |
| `bukio import xaf --file F [--dry-run]` | XML Auditfile Financieel 4.0 import — Belastingdienst OR generic AuditFile layout; file chart upserted (renames colliding codes on an empty ledger, shown in the dry-run); KVK mismatch fails `COMPANY_MISMATCH`. |
| `bukio import invoice --file F [--contact N] [--create-missing] [--dry-run]` | Import an inbound e-invoice (EN 16931/Peppol BIS 3.0 UBL) into the **payables register** — whole-file validation, idempotent by supplier+invoice number, VAT breakdown reported but NOT booked (book via the normal workflow). Credit notes (381) rejected. |
| `bukio import contacts --file F [--dry-run]` | Import suppliers + customers from an audit file as invoice contacts (idempotent by name; whole-file validation). |
| `bukio export xaf --year YYYY --out file.xaf` | Export the fiscal year as an **Auditfile Financieel 4.0 XML** (Belastingdienst format) — the file a boekhouder/tax advisor/auditor can pull straight into their own software. Posted entries only; drafts are excluded. Round-trips through `import xaf`. |
| `bukio month-end --period yyyy-mm` | **Read-only** close check: drafts, unmatched bank, OB quarter readout, draft/overdue invoices, due recurring, period totals + profit, `warnings[]`. |
| `bukio invoice reminders [--within-days N] [--draft-emails]` | Overdue + due-soon invoices (most overdue first) with outstanding amounts; `--draft-emails` adds Dutch email drafts — nothing is sent. |
| `bukio assets scheme add --name [--method lineair\|degressief] [--life-months 60] [--residual-bp 0]` | Depreciation schemes; the standard 5y-lineair-0% scheme is created lazily when an asset has no scheme. |
| `bukio assets add --name --purchase-date --purchase-price --depreciation-start --recognition-date [--cum-dep] [--scheme \| --method/--life-months] [--asset-account 1800] [--cum-dep-account] [--expense-account 4600] [--entry-id]` | Register an already-booked asset (mid-life adoption: only remaining depreciation runs forward, first run on the 1st). GL reconciliation warnings. |
| `bukio assets run [--period yyyy-mm \| --as-of DATE] [--dry-run]` | Book due depreciation (source 'assets', idempotent per asset-month, auto-completes at residual). |
| `bukio assets register [--as-of DATE] [--format json\|csv\|xlsx --out]` | Activastaat: cost / cum-dep / book value per asset + totals. |
| `bukio assets dispose --id N --date [--proceeds] [--bank-account] [--result-account] [--dry-run]` | Sale/scrap: proposes the full entry, status → disposed. |
| `bukio assets list/show/pause/resume` | Register inspection + pause/resume. |
| `bukio payments payables add --contact N --ref --date --amount [--due] [--method transfer\|direct-debit] [--entry-id]` | Register a purchase invoice. `direct-debit` = incasso (collected by the vendor — only joins direct-debit batches). |
| `bukio payments payables list [--status] [--method]` / `pay --id` | Open payables; `pay` marks paid after the bank statement confirms. |
| `bukio payments batch create [--type transfer\|direct-debit] [--from-invoices] [--payable ids] [--lines "C:AMT[:REF];…"] [--csv file] [--date] [--from-iban] [--dry-run]` | Build a batch from unpaid payables matching the type and/or explicit lines. Whole-set validation (IBAN mod-97, amounts > 0, refs ≤ 140); direct-debit lines get FRST/RCUR + the mandate snapshot automatically. |
| `bukio payments mandate add --contact N --ref R [--date D] [--type core\|b2b]` / `list [--contact]` / `remove --id` | SEPA incassovolmacht register. `b2b` = no refund right. Required before a direct-debit batch can collect from a contact. |
| `bukio payments batch export --id N [--schema 001.03\|001.09] [--out file.xml] [--dry-run]` | SEPA XML for bank-portal upload: pain.001 for transfer, **pain.008.001.02** for direct-debit (one `PmtInf` per CORE/B2B scheme). **Once per batch** — the MsgId is stored; re-export would double-pay/collect. |
| `bukio payments batch list [--status]` / `show --id` / `delete --id` | Batch tracking; delete only on drafts (releases payables to unpaid). |
| `bukio contact update --id N [--iban …]` / `contact add --iban` | Contact IBANs (mod-97) — required before a vendor can join a batch. |
| `bukio backup [--out PATH]` / `bukio restore --from FILE [--force]` | Consistent backup / validated restore. **Database only — NOT the original documents** (see 6.18). Backups are manual, not automatic. |
| `bukio update [--yes] [--repo PATH] [--trust-remote] [--dry-run]` | Self-update from the GitHub main branch: fetches `origin/main` and resets the working tree to it (audit row when a company DB exists). **Reset --hard overwrites local customizations** — `--dry-run` first (shows incoming commits + what would be lost), and the real run refuses without `--yes`. `--trust-remote` for forks/mirrors; `--repo` for a different install. |
| `bukio audit [--by agent:bartholomeus] [--since ISO] [--limit N] [--format json\|csv\|xlsx] [--out PATH]` | Read the append-only audit log (newest first); export to csv/xlsx for an external advisor. |
| `bukio audit verify [--since ISO] [--limit N]` | Re-verify the signed audit trail against the company registry: recompute the canonical digest from the stored signed args and check every signature. Per-row `ok | unsigned | revoked | tampered | invalid-signature | unknown-key`; a summary with counts. Reads only the DB (a copied file verifies self-contained). Exit 1 when tampered/invalid/unknown-key rows exist. |
| `bukio actor keygen [--force] [--dry-run]` | Generate an Ed25519 keypair for the `--actor`: human keys are passphrase-encrypted (`BUKIO_SIGNING_PASSPHRASE` or interactive prompt), agent/system keys are plain files. Written to `<config>/keys/<role>-<name>.key` (0600). `--force` replaces an existing key (rotation). |
| `bukio actor register [--dry-run]` | Enrol the actor's local key into the **current company's DB** (`actor_keys`). Per-company: repeat for every company DB the actor works in. First enrolment is refused under enforcement (operator-gated: an enrolled actor flips `enforce --off` → register → `--on`); re-enrolment after revocation (rotation) works under enforcement. |
| `bukio actor list` | List enrolled actors in the current company's DB (active/revoked + reason). |
| `bukio actor revoke --reason TEXT [--target <who>] [--dry-run]` | Revoke an actor's key in the current company's DB. The row is **retained** (with reason) so historical audit verification still works; the key stops authorising new commands. Default: your own key. `--target <who>` is the OWNER-mediated kill of a compromised key — requires the owner role **regardless of authz mode**. |
| `bukio actor enforce --on\|--off [--dry-run]` | Toggle signed-command enforcement for this company (audited). `record` mode (default) verifies signatures when present; `enforce` mode refuses unsigned/invalid commands. **Only an enrolled actor can turn `--off`** — the audited recovery valve. |
| `bukio actor authz --on\|--off [--dry-run]` | Toggle per-actor authorizations (segregation of duties) for this company. `--on` **implies signing enforcement** and grants the flipper the `owner` role (bootstrap — no deadlock); `--off` needs the owner role (enforcement stays on). Under authz, every non-exempt command needs a role granting its capability — **deny-by-default**, refused with `AUTHZ_DENIED` before anything is written (dry-run included). |
| `bukio actor roles [--for <who>]` / `roles grant <role> --for <who> [--dry-run]` / `roles revoke <role> --for <who> [--dry-run]` | Role grants. Listing your OWN roles is self-service; seeing others' roles, granting and revoking are owner-only under authz (inert configuration while authz is off). Grants warn — softly, the owner decides — on segregation-of-duties conflicts (`bookkeeper`+`payments`, `bookkeeper`+`tax`, `payments`+`tax`, `entry.post`+`payments.sepa`). The LAST owner can never be revoked. Roles: `owner\|bookkeeper\|payments\|tax\|assets\|readonly`. |
| `bukio actor can '<cmd>' [--for <who>]` | Capability check: `ok` or `AUTHZ_DENIED` with the missing capability. Self-service for your own checks; `--for <who>` is owner-only under authz. Accepts `entry add --post` (the actual mutation decides: `entry.post` vs `entry.draft`) and MCP form `mcp:entry_add`. |
| `bukio actor who-can '<cmd>'` | Which actors can run a command (the SoD review lens; owner-only under authz). |
| `bukio actor unlock [--ttl-hours N] [--dry-run]` / `actor lock` | Human-only: decrypt the passphrase-encrypted key into a short-lived session key (`<config>/sessions/<role>-<name>.key`, default 12 h). `lock` clears it early. |
| `bukio actor verify` | Check an actor's key state (key file present, keyid, registered/active/revoked) against the current company's registry. |

**Import contract (all three importers):** the ENTIRE file is validated before
anything is written. Any problem → `IMPORT_VALIDATION_FAILED` with
`error.details: [{ line, error }]` and NOTHING is imported. Idempotent:
re-imports skip already-imported boekstukken (`duplicates` in the result).
Import amounts accept `1234.56`, `1234,56` and `1.234,56`; `;`-delimited files
split on `;` only (decimal commas stay intact). Journal/XAF `BtwCode` values
are reported in `ignored_btw_codes` but NOT booked — the import is net; verify
the booked amounts.

**Posting syntax:** `--postings "1100:10000.00,3000:-10000.00"` — comma-separate or repeat the flag. `CODE` is the 4-digit account code. **Positive = debit, negative = credit. Sum must be zero.** Amount format: `1234.56`, max 2 decimals, no thousands separators, no Dutch comma decimals.

---

## 4. JSON contracts

### Success

```jsonc
{ "ok": true, "data": { ... } }
```

### Failure

```jsonc
{ "ok": false, "error": { "code": "UNBALANCED", "message": "postings do not sum to zero (sum = 1 cents)" } }
```

### `entry add` (with `--post`) → `data`

```jsonc
{
  "id": 1,
  "date": "2026-08-04",
  "description": "Startkapitaal",
  "source": "manual",
  "source_ref": null,
  "state": "posted",            // "draft" if --post omitted
  "reversed_from_id": null,
  "created_by": "agent:bartholomeus",
  "created_at": "2026-08-04T19:09:10.067Z",
  "posted_at": "2026-08-04T19:09:10.068Z",
  "postings": [
    { "id": 1, "account_code": "1100", "account_name": "Bank", "account_type": "asset",
      "amount_cents": 1000000, "amount": "10000.00" },
    { "id": 2, "account_code": "3000", "account_name": "Eigen vermogen", "account_type": "equity",
      "amount_cents": -1000000, "amount": "-10000.00" }
  ]
}
```

### `report trial-balance` → `data`

```jsonc
{
  "year": null,
  "accounts": [
    { "code": "1100", "name": "Bank", "type": "asset",
      "debit_cents": 1000000, "credit_cents": 25000, "net_cents": 975000,
      "debit": "10000.00", "credit": "250.00", "net": "9750.00" }
  ],
  "total_debit_cents": 1025000,
  "total_credit_cents": 1025000,
  "total_debit": "10250.00",
  "total_credit": "10250.00",
  "balanced": true
}
```

### `audit` → `data.entries[]`

```jsonc
{
  "id": 5,
  "ts": "2026-08-04T19:09:10.272Z",
  "actor": "agent:bartholomeus",
  "action": "entry.post",           // company.init | entry.create | entry.post | entry.reverse
  "command": "entry post",
  "args_json": "{\"id\":2}",
  "args": { "id": 2 },
  "outcome": "ok",
  "entry_ids": [2]
}
```

---

## 5. Account codes (default chart — 28 accounts, RGS-mapped)

| Code | Name | Type | RGS |
|------|------|------|-----|
| 1000 | Kas | asset | BLIM.10 |
| 1100 | Bank | asset | BLIM.10 |
| 1200 | Debiteuren | asset | BVOR.11 |
| 1400 | Voorraad | asset | BVRD.30 |
| 1600 | Overige vorderingen | asset | BVOR.11 |
| 1700 | Vooruitbetaalde kosten | asset | BVOR.11 |
| 1800 | Materiële vaste activa | asset | BMVA.02 |
| 1850 | Vervoermiddelen | asset | BMVA.02 |
| 2000 | Crediteuren | liability | BSCH.12 |
| 2100 | Overige schulden | liability | BSCH.12 |
| 2300 | Vooruitontvangen bedragen | liability | BSCH.12 |
| 2400 | Nog te betalen kosten | liability | BSCH.12 |
| 2900 | Rekening-courant | liability | BSCH.12 |
| 3000 | Eigen vermogen | equity | BEIV.05 |
| 4000 | Inkoopwaarde | expense | WKPR.70 |
| 4100 | Huisvestingskosten | expense | WBED.42 |
| 4200 | Autokosten | expense | WBED.42 |
| 4300 | Kantoor- en algemene kosten | expense | WBED.42 |
| 4310 | Accountants- en administratiekosten | expense | WBED.42 |
| 4320 | Verzekeringen | expense | WBED.42 |
| 4330 | Telecommunicatie | expense | WBED.42 |
| 4340 | Software en internetdiensten | expense | WBED.42 |
| 4400 | Personeelskosten | expense | WPER.40 |
| 4500 | Financiële baten en lasten | expense | WFBE.84 |
| 4600 | Afschrijvingen | expense | WAFS.41 |
| 4700 | Overige bedrijfskosten | expense | WBED.42 |
| 8000 | Omzet | income | WOMZ.80 |
| 8100 | Overige opbrengsten | income | WOVB.82 |

`rgs_code` is the RGS hoofdgroep (niveau 2) reference — balans and P&L group by it. Accounts you add with an unknown/empty `rgs_code` land in an "Overig" section (still counted — never silently dropped).

When the **VAT module is enabled**, two accounts are added: **1500 Te vorderen omzetbelasting** (asset, BVOR.11) and **2500 Te betalen omzetbelasting** (liability, BSCH.12), plus 8 VAT codes (21, 9, 0, V vrijgesteld, R/RE verlegd, M marge, P privé). Use `vat book` with `@CODE` tags — never construct VAT entries by hand; the module computes the VAT leg and validates it.

There are **no VAT accounts** in the core chart — VAT is an optional module. When the module is on, VAT accounts and codes are added; until then, book amounts exclusive of VAT or use `2100` for balances payable (see worked example 6.5 for the shape of a VAT-style 3-leg entry, which works today).

---

## 6. Worked examples (copy-paste patterns)

### 6.1 Open the month / company start

```bash
BUKIO_DB=$HOME/.bukio/demo.db bukio init --name "Demo BV" --kvk 12345678 --legal-form bv --vat on --dry-run
BUKIO_DB=$HOME/.bukio/demo.db bukio init --name "Demo BV" --kvk 12345678 --legal-form bv --vat on
BUKIO_DB=$HOME/.bukio/demo.db bukio entry add --desc "Startkapitaal" \
  --postings "1100:10000.00,3000:-10000.00" --post --actor agent:bartholomeus --json
```

### 6.2 Book an expense paid from the bank

```bash
bukio entry add --desc "Kantoorartikelen" --postings "4300:250.00,1100:-250.00" --post
```

### 6.3 Book sales

```bash
bukio entry add --desc "Factuur 2026-001" --postings "1100:1210.00,8000:-1210.00" --post
```

### 6.4 Correct a mistake (reverse + re-book)

```bash
# 1. always dry-run the reversal first
bukio entry reverse --id 2 --reason "verkeerde categorie" --dry-run --json
# 2. apply it (original stays posted; contra-entry cancels it)
bukio entry reverse --id 2 --reason "verkeerde categorie" --actor agent:bartholomeus --json
# 3. book the corrected entry
bukio entry add --desc "Kantoorartikelen (gecorrigeerd)" --postings "4200:250.00,1100:-250.00" --post
# 4. verify the books
bukio report trial-balance --json   # balanced: true
```

### 6.5 3-leg entry (net amount + tax liability split — works today, VAT module later)

```bash
bukio entry add --desc "Factuur met btw 21%" \
  --postings "1100:121.00,8000:-100.00,2100:-21.00" --post --dry-run
```

### 6.6 Month-end close: bank import -> match -> VAT readout (the standard loop)

```bash
# 1. books must reconcile (before touching the bank)
bukio report trial-balance --json          # data.balanced === true
# 2. import the bank statement — CAMT.053 from your bank, or the bank CSV export
bukio bank import --file ~/exports/rabo-2026-06.xml --iban NL91ABNA0417164300 --dry-run
bukio bank import --file ~/exports/rabo-2026-06.xml --iban NL91ABNA0417164300
# 3. auto-match against already-booked entries (dry-run first!)
bukio bank match auto --dry-run --json
bukio bank match auto --json
# 4. handle leftovers: suggestions -> post (creates + reconciles the entry) or link
bukio bank match suggest --json
bukio bank match post --tx 17 --account 4300 --dry-run
bukio bank match post --tx 17 --account 4300
# 5. verification: bank balance == ledger bank balance, books balanced
bukio bank list --json
bukio report trial-balance --json          # data.balanced === true
# 6. VAT quarter (module on): read the OB fields, human files manually
bukio vat readout --period 2026-Q2 --json
bukio vat readout --period 2026-Q2 --mark-filed
# 7. export for the boekhouder + backup
bukio report journal --year 2026 --format xlsx --out ~/exports/journal-2026.xlsx
bukio backup --json
```

### 6.7 Book a VAT-aware sale and purchase

```bash
# sale 121.00 incl 21%: omzet 100 + te betalen btw 21 (auto-computed)
bukio vat book --date 2026-06-01 --desc "Factuur 2026-001" \
  --postings "1100:121.00,8000:-100.00@21" --post --dry-run
# purchase 60.50 incl 21%: kosten 50 + te vorderen btw 10.50
bukio vat book --date 2026-06-05 --desc "Kantoorartikelen" \
  --postings "4300:50.00@21,1100:-60.50" --post
# reverse charge: net 100, VAT self-assessed (auto @R feeds 4a/5b; no VAT leg
# is booked — verschuldigd + aftrekbaar net to zero, the supplier never
# charged VAT, so the bank leg stays at the net amount)
bukio vat book --date 2026-06-06 --desc "Inkoop verlegd" \
  --postings "4300:100.00@R,1100:-100.00" --post
```

### 6.8 Extend the chart

```bash
bukio account add --code 4350 --name "Reiskosten" --type expense --normal-balance debit --rgs-code WBED.42 --dry-run
bukio account import --file assets/chart-nl.csv --dry-run   # validate; then import without --dry-run
```

### 6.9 Report on what an agent did

```bash
bukio audit --by agent:bartholomeus --json
```

### 6.10 Month-end: run the recurring templates (depreciation, accruals)

```bash
# 1. what is due (read-only — never guess, always preview first)
bukio recurring preview --as-of 2026-09-30 --json
# 2. generate (backfills missed periods; dry-run first)
bukio recurring run --as-of 2026-09-30 --dry-run --json
bukio recurring run --as-of 2026-09-30 --json
# 3. verify the books
bukio report trial-balance --json          # data.balanced === true
# 4. depreciation schedules: 5370.00 / 36 mnd -> 149.17/mo, final 149.05
bukio depreciation add --name "Laptop Dell" --cost 5370.00 --life-months 36 \
  --start 2026-08-01 --dry-run
# 5. accrual with auto-reversal (each run reverses the previous estimate)
bukio recurring add --name "Nog te betalen kosten admin" \
  --postings "4310:250.00,2400:-250.00" --frequency monthly \
  --start 2026-08-31 --day 28 --reverse-previous
```

Generated entries are `source='recurring'`, `created_by='recurring'` — never hand-edit
them; pause the template (`recurring pause --id`) if a schedule must stop, and reverse
individual generated entries with `entry reverse` if one is wrong (the template's
`last_entry_id` then stays pointing at the reversal-safe state).

---

### 6.11 Invoice a client, get paid, correct a mistake

```bash
# 1. the supplier must be complete (init) and the customer registered
bukio contact add --name "ACME B.V." --address "Straat 1" --postal-code "1000 AA" \
  --city "Amsterdam" --vat-id NL999999999B01
# 2. draft -> finalize (validates the 12 factuurvereisten, books the entry)
bukio invoice create --contact 1 --date 2026-07-10 \
  --lines "2x Consultancy @ 150.00 @21,1x Rapportage @ 400.00 @9" --dry-run
bukio invoice create --contact 1 --date 2026-07-10 \
  --lines "2x Consultancy @ 150.00 @21,1x Rapportage @ 400.00 @9"
bukio invoice finalize --id 1 --dry-run     # plan: 2026-0001 + postings
bukio invoice finalize --id 1
# 3. deliverables
bukio invoice pdf --id 1                     # 2026-0001.pdf (send to the client)
bukio invoice ubl --id 1                     # 2026-0001.xml (Peppol BIS 3.0)
# 4. payment arrives -> bank match closes the loop
bukio bank import --file stmt.xml --iban NL91ABNA0417164300
bukio bank match auto --json                 # tx -> invoice 2026-0001 (paid)
# 5. mistake -> credit note (reversal entry), never delete
bukio invoice credit --id 1 --reason "verkeerd tarief"
bukio invoice finalize --id 2                # 2026-0002, reverses the booking
```

Booking rule: finalized invoices auto-post Debiteuren/Omzet/Te betalen btw (per-rate,
line-exact VAT). Payments post Bank/Debiteuren when matched from the bank. Do not
hand-construct invoice entries with `entry add` — finalize/credit do it correctly.

### 6.12 Year-end: close the books, produce the jaarrekening

```bash
# 1. everything must be posted before closing (drafts block the close)
bukio year-end status --year 2026
bukio year-end close --year 2026 --dry-run     # plan: result + postings
bukio year-end close --year 2026               # result -> 9900 -> 3000
# 2. statutory accounts + KVK deposit package
bukio financial-statements report --year 2026 --model klein --format pdf   # financial-statements-2026-klein.pdf
# 3. quarterly EU listing (verlegde EU leveringen)
bukio icp readout --period 2026-Q3
# 4. the OB readout now also reports 2a (EU) and 3b (EU inkopen)
bukio vat readout --period 2026-Q4
```

Closing rules: `source='closing'` entries are excluded from the P&L (the year's
flow stays visible) but included in the balans (equity carries the result).
Undo a close with `entry reverse` on the closing entries — never by deleting
them. The OB readout tracks 2a (verlegde EU sales), 3a/3b (verlegde inkopen)
and 1d (privégebruik at 21%); 2b and 5c are not tracked.

### 6.13 Foreign-currency purchase invoices (FX)

```bash
# 1. store the rate once (upsert; audited) — or let the ECB provide it
bukio fx set --currency USD --date 2026-07-03 --rate 1.0875
bukio fx fetch --currency GBP --date 2026-08-03          # ECB, stored as source=ECB
# 2. book the invoice: amounts in the specs are USD, the ledger stores EUR.
#    Missing rates are auto-fetched from the ECB (one call ever, then reused)
bukio vat book --date 2026-08-01 --desc "Stripe Inc. - INV-9102 (USD)" \
  --currency USD --postings "4300:895.00@21,1100:-1082.95" --dry-run
bukio vat book --date 2026-08-01 --desc "Stripe Inc. - INV-9102 (USD)" \
  --currency USD --postings "4300:895.00@21,1100:-1082.95" --post --actor agent:bartholomeus
# 3. koersverschil at payment (the bank shows a different EUR amount): book the
#    difference on 4700 Koersverschillen (create the account first)
bukio account add --code 4700 --name "Koersverschillen" --type expense --normal-balance debit
```

FX rules: the rate resolves as `--rate` -> stored rate (exact, else latest
on/before) -> **ECB reference rate** (auto-fetched, stored as source=ECB for
reuse; `BUKIO_FX_NO_FETCH=1` disables the network). Conversion is round-half-up
integer math; postings carry `fx_currency`/`fx_amount_cents` so the original
amount is always auditable; reversals negate both; VAT legs are EUR-only.
Outgoing invoices stay EUR-only. Never hand-build FX entries — always
`--currency` or a converted spec.

### 6.14 Migrate from an old package + monthly close check

```bash
# 1. opening balances first (validates the WHOLE file, then posts ONE entry)
bukio import opening-balances --file beginbalans.csv --date 2026-01-01 --dry-run
bukio import opening-balances --file beginbalans.csv --date 2026-01-01
# 2. journal CSV (SnelStart/Exact-style; unknown accounts with --create-missing)
bukio import journal --file snelstart-journal.csv --create-missing --dry-run
bukio import journal --file snelstart-journal.csv --create-missing
# 3. the Belastingdienst audit format
bukio import xaf --file audit.xaf --dry-run
bukio import xaf --file audit.xaf
# 4. every month, the agent runs the close check and the reminder list
bukio month-end --period 2026-08 --json
bukio invoice reminders --within-days 7 --draft-emails --json
```

Import failures carry per-line details:
`error.details = [{ line: 2, error: "ACCOUNT_NOT_FOUND: account 9999 …" }]`.
Fix the file, re-run — already-imported boekstukken are skipped, nothing is
double-booked. Never "fix" a bad import by reversing parts of it — correct the
file and re-import (idempotency handles the rest).

### 6.15 Fixed assets: mid-life adoption + the monthly run

```bash
# 1. recognise an asset bought years ago (already booked in the ledger):
#    cost 1511.74, depreciation started 2024-01-01, recognised now with
#    989.60 already depreciated -> only the REMAINING 522.14 runs forward
bukio assets add --name "Laptop" --category ICT --purchase-date 2023-12-20 \
  --purchase-price 1511.74 --depreciation-start 2024-01-01 \
  --recognition-date 2025-12-31 --cum-dep 989.60 \
  --asset-account 1350 --cum-dep-account 1500 --expense-account 4600 --dry-run
bukio assets add ...  # (same, without --dry-run; GL reconciliation warnings ok)
# 2. every month (or caught by `month-end`): book what is due
bukio assets run --period 2026-01 --dry-run
bukio assets run --period 2026-01
# 3. the activastaat for the jaarrekening
bukio assets register --as-of 2026-12-31 --format xlsx --out activastaat.xlsx
# 4. selling/scrapping: propose the full entry (bank/cum-dep/asset/result)
bukio assets dispose --id 1 --date 2026-06-15 --proceeds 450.00 --dry-run
```

Assets are `source='assets'` with `source_ref='asset:<id>:<YYYY-MM>'`; runs are
idempotent per asset-month and auto-complete the asset at its residual.

### 6.16 SEPA payment batches: payables → pain.001 → bank portal

```bash
# 1. give vendors an IBAN (mod-97 validated); company needs its IBAN too
bukio company update --iban NL91ABNA0417164300
bukio contact update --id 3 --iban NL02ABNA0123456789
# 2. register purchase invoices. 'direct-debit' (incasso) payables are
#    collected by the vendor and stay OUT of batches
bukio payments payables add --contact Vimexx --ref 2026-118 --date 2026-07-01 \
  --due 2026-08-01 --amount 121.00
bukio payments payables add --contact "Energie BV" --ref 2026-09 --date 2026-07-05 \
  --amount 99.99 --method direct-debit
# 3. select what to pay: all unpaid transfer payables, or specific ones
bukio payments batch create --from-invoices --dry-run      # plan
bukio payments batch create --from-invoices                # batch #1, payables -> in_batch
# 4. export the SEPA file (unique MsgId; export only ONCE per batch)
bukio payments batch export --id 1 --schema 001.03 --out betalingen.xml
#    -> upload betalingen.xml in the bank portal; the bank executes it
# 5. after the bank statement arrives: import + match as usual, then
bukio payments payables list                               # see what is still open
bukio payments payables pay --id 1                         # confirm paid
```

The export books NOTHING — money moves when the bank processes the file; the
CAMT.053 import closes the loop. Deleting a draft batch releases payables back
to `unpaid`. Never re-export a batch (the stored MsgId guard exists because
re-uploading the same file would pay twice).
`assets add` never re-books the purchase or past depreciation — recognition is
registration-only (warnings, not errors, when the GL accounts don't reconcile).

### 6.17 Archive source documents (mandatory after every booking)

```bash
# every booked invoice's original lives next to the DB
mkdir -p ~/.bukio/invoices
cp <original>.pdf ~/.bukio/invoices/2026-07-10_acme-bv_F2026-123.pdf
# naming: YYYY-MM-DD_<vendor-slug>_<invoice-number>.<ext>
#   - date = invoice date (sortable, findable by month)
#   - vendor-slug = lowercase supplier name, dashes for spaces
#   - invoice-number = the supplier's reference, verbatim
#   - extension = original format (.pdf, .jpg, .png, .xml, .eml, ...)
# other formats welcome — keep whatever arrived, don't convert
ls -la ~/.bukio/invoices/        # verify it landed
```

Rules:
- The archive lives at `~/.bukio/invoices/` (next to the live DB `~/.bukio/bukio.db`).
- Per-company databases get their own archive next to their DB (e.g. `~/.bukio/demo.db`
  → `~/.bukio/demo-invoices/`).
- Archive **before** finishing the booking, so a crash mid-booking never loses the paper.
- The journal entry description should carry the same reference
  (`"Vendor - F2026-123"`), so entry ↔ archive lookup is one grep.

> ⚠️ **Archiving is NOT a backup.** Copying a document into `~/.bukio/invoices/`
> only keeps it with the books — it does not protect it against disk loss.
> See 6.18 for what a real backup covers (and does not).

### 6.18 Backups are manual — database AND documents (not automatic)

Nothing in bukio backs itself up automatically. **The agent or the user must
take backups periodically** — there is no scheduler, no auto-archive job, and
no off-site copy. If the disk dies, everything since the last manual backup is
gone. Treat backups as part of the monthly close (6.6) and after any large
import or year-end.

```bash
# 1. the DATABASE — `bukio backup` takes a consistent snapshot of the ledger
#    (SQLite backup API: safe even mid-write). This is the ONLY thing it covers.
bukio backup --out ~/backups/bukio-2026-08-06.db --json

# 2. the ORIGINAL DOCUMENTS — `bukio backup` does NOT include these.
#    Copy the invoice archive (and any exports) yourself:
tar -czf ~/backups/bukio-invoices-2026-08-06.tar.gz -C ~/.bukio invoices/
#    or plain rsync/cp — anything that copies the files off the live disk.
```

What `bukio backup` covers:
- ✅ the SQLite database (accounts, entries, postings, audit log, everything)
- ❌ **not** the original PDFs/images in `~/.bukio/invoices/`
- ❌ not exports (`--out` files), jaarrekening PDFs, or SEPA files you generated

Suggested cadence (agent should propose, user approves):
- **Monthly**, at the month-end close (6.6): `bukio backup` + archive the
  invoice directory that month's documents landed in.
- **After big events**: a large import (XAF/journal), year-end close, or any
  bulk mutation — backup immediately after.
- **Off-site**: keep at least one copy on a different machine or medium than
  the DB (cloud storage, external disk, second server). A backup on the same
  disk as the DB is not a backup.

Restore: `bukio restore --from <file> --force` (validated). Restoring does not
touch the invoice archive — documents must be restored from your manual copy.

### 6.19 Hand the year to an external advisor (XAF + audit export)

When a boekhouder, tax advisor or auditor needs to look at the books, export
the year as an Auditfile Financieel 4.0 XML — the Belastingdienst standard that
SnelStart, Exact and other accounting packages import directly:

```bash
# 1. the audit file: company header + chart + one Mutatie per posted entry
bukio export xaf --year 2026 --out ~/exports/bukio-2026.xaf
# 2. the audit trail as a spreadsheet (who did what, when)
bukio audit --format xlsx --out ~/exports/bukio-audit-2026.xlsx --limit 1000
# 3. hand over the source documents too (see 6.17 for the archive convention)
tar -czf ~/exports/bukio-documenten-2026.tar.gz -C ~/.bukio invoices/
```

`export xaf` semantics:
- **Posted entries only** — drafts are not part of the books as they stand and
  are excluded (an export of a year with only drafts fails `EXPORT_EMPTY_YEAR`).
- One `<Mutatie>` per entry, `<Boekstuknummer>` = the entry id (stable,
  unique); postings become `<Boeking>` pairs (debet vs credit) exactly like the
  importer reads them — **an exported file re-imports losslessly**.
- Read-only: it writes the file and one `export.xaf` audit row; nothing else
  is touched.

### 6.20 Signing & actor identity: what YOU do, when it signs

Signing is **automatic once you are set up** — you never type a signature.
This is the whole workflow:

**One-time setup (per identity, per machine).** Create a key for every
identity you act as, then enrol it in every company DB that identity works
in:

```bash
# as the human owner — the key is passphrase-encrypted
bukio --actor human:erik actor keygen      # prompts for a passphrase
bukio --actor human:erik actor register    # enrol into THIS company's DB
# repeat `actor register` for every other company DB the human works in

# as an agent / system — the key is a plain file
BUKIO_ACTOR=agent:bartholomeus bukio actor keygen
BUKIO_ACTOR=agent:bartholomeus bukio actor register   # per company DB
BUKIO_ACTOR=system:backup bukio actor keygen
BUKIO_ACTOR=system:backup bukio actor register
```

**Per session (humans only).** `human:` keys are encrypted at rest; unlock
the key for the session (default 12 h), `actor lock` clears it early:

```bash
bukio --actor human:erik actor unlock
```

**That's it.** From now on **every** command you run — reads *and* writes,
`--dry-run` included — is signed automatically before it executes:

- the exact command + arguments + timestamp + a fresh nonce are turned into
  a canonical digest,
- the digest is signed with the actor's key (resolution order: `--sign-key`
  → session → `BUKIO_SIGNING_PASSPHRASE` → `<config>/keys/<role>-<name>.key`),
- the signature is verified against the company's `actor_keys` registry and
  stored on the audit row for that action (`sig_status` = `verified`).

You can watch it happen: `bukio audit --json` shows `sig_status` per row,
and `bukio actor verify` shows your key state (key file? enrolled? session?
enforcement?).

**Enforcement.** Default (`record` mode): unsigned commands still run and
log `unsigned` — harmless, just not provable. When a company has
`actor enforce --on`: unsigned, unverifiable, stale, replayed, unknown-key
or revoked-key commands are **refused before anything is written** — the
audit row records the attempt. Enforcement is per company: your key must be
registered in *that* company's DB.

**When it breaks — the error tells you what to do:**

| Refusal | Meaning | Fix |
|---------|---------|-----|
| `SIGNATURE_REQUIRED` | no key material found | `actor keygen` (+ `unlock` for humans) |
| `ACTOR_KEY_UNKNOWN` | key exists, not enrolled in this company | `actor register` (first enrolment: operator flips `enforce --off` → register → `--on`) |
| `ACTOR_KEY_REVOKED` | key was revoked here | rotate: `actor keygen --force` + `actor register` |
| `PASSPHRASE_REQUIRED` | encrypted human key, no session/passphrase | `actor unlock` or set `BUKIO_SIGNING_PASSPHRASE` |
| `AUTHZ_DENIED` | authorizations on, actor lacks the capability (or the command is unmapped — fail closed); also `actor revoke --target` without the owner role | `actor roles` (own roles), `actor can '<cmd>'` (why), ask the owner to `roles grant` |

### 6.21 Authorizations & segregation of duties (Tier 0.5)

Signing proves **who** ran a command; authorizations control **what kinds
of commands** an actor may run, so one actor can no longer do both sides of
a sensitive pair (create *and* post, book *and* file tax, move the ledger
*and* authorise money out). The granularity is **capability families +
roles**, not a per-command matrix — one grant line the owner can read:

| Role | Capabilities | Typical holder |
|------|--------------|----------------|
| `owner` | **all** | `human:erik` |
| `bookkeeper` | admin.chart, entry.draft, entry.post, contacts.manage, invoice.manage, bank.import, bank.match, vat.book, assets.manage, recurring.manage, fx.manage, report.read | `human:erik`, `agent:bookkeeping` |
| `payments` | bank.import, bank.match, payments.sepa, report.read | `agent:payments` |
| `tax` | vat.book, vat.file, close.month, close.year, export.manage, report.read | `agent:tax` |
| `assets` | assets.manage, report.read | `agent:assets` |
| `readonly` | report.read | auditor, `system:watchdog` |

(There is also a documented *automation template* for working agents:
entry.draft, contacts.manage, invoice.manage, bank.import,
recurring.manage, report.read — the operator adds `entry.post`
deliberately when the agent is trusted to book.)

**Enable it (one-time, audited).** `actor authz --on` turns on signing
enforcement (it implies it — permissions without proven identity are
meaningless) and **grants the flipper the `owner` role** (bootstrap — no
deadlock, no pre-seeding). Enrol every agent's key **before** flipping:

```bash
# enrol the agents first (first enrolment is refused under enforcement)
BUKIO_ACTOR=agent:invoicing bukio actor keygen && bukio actor register
BUKIO_ACTOR=agent:payments  bukio actor keygen && bukio actor register

# flip: the flipper becomes owner, enforcement switches on
bukio --actor human:erik actor authz --on

# grant roles (owner only under authz); a conflicting grant warns softly
bukio actor roles grant bookkeeper --for agent:invoicing
bukio actor roles grant payments    --for agent:payments
```

> `--for <who>` names the actor a role goes to. The identity flag
> `--actor` is the global option — it cannot be re-used for the grantee
> (a subcommand option with the same name would be silently bound to the
> global and corrupt the signing identity), so the grantee flag is `--for`.

**What changes.** Under authz, **every** non-exempt command needs a role
carrying its capability — deny-by-default. Refusals happen in the sign
gate, **before anything is written** (dry-run included):

- `AUTHZ_DENIED` names the actor, the missing capability and the roles it
  has: `actor agent:invoicing has no capability 'vat.file' in this company
  (roles: bookkeeper) — ask the owner to grant it`.
- Self-service stays free for any enrolled actor: `actor verify`,
  `actor can '<cmd>'` (your own checks), `actor roles` (your own), and
  revoking your own key. Seeing **others'** roles (`roles --for`),
  `actor can --for`, `actor who-can`, granting/revoking roles, and
  `authz --off` are owner territory.
- The owner-mediated key kill: `actor revoke --target <who> --reason` —
  owner role required **regardless** of authz mode (a compromised agent
  key is killed by the operator, not only by itself).

**Review the separation (SoD).** The review lens is `who-can` — the whole
matrix for one command:

```bash
bukio actor who-can 'entry post'          # only owner + bookkeeper
bukio actor who-can 'payments batch create'  # only owner + payments
```

**Turn it off.** `actor authz --off` (owner only) returns to Tier 0 —
signing enforcement stays on, roles become inert configuration. The LAST
owner can never be revoked (`LAST_OWNER`), so a company can always turn
authz off.

## 7. Error codes you will meet

| Code | What it means | What to do |
|------|---------------|------------|
| `NO_DATABASE` | DB file missing | Run `bukio init` first |
| `ALREADY_INITIALISED` | Company exists | Point `--db` at a fresh file |
| `MIGRATION_FAILED` | A schema migration could not be applied (malformed SQL, version conflict) | The database is left unchanged (transaction rolled back); report the migration file + error |
| `INVALID_AMOUNT` | Bad amount string | Use `1234.56`, max 2 decimals, no separators |
| `INVALID_POSTING` | Spec not `CODE:AMOUNT` | Fix the spec |
| `TOO_FEW_POSTINGS` | < 2 postings | Add the counter-posting |
| `UNBALANCED` | Sum != 0 | Debits must equal credits |
| `ACCOUNT_NOT_FOUND` / `ACCOUNT_INACTIVE` | Bad code | Use codes from §5 |
| `NOT_FOUND` | Bad entry id | `bukio entry list` to find it |
| `ALREADY_POSTED` | Entry posted | Nothing to do, or reverse it |
| `NOT_POSTED` | Entry is draft | `entry post` first, then reverse |
| `ALREADY_REVERSED` | Reversal exists | Find the contra-entry via `entry list` / `entry show` |
| `SQLITE_CONSTRAINT_TRIGGER` | You violated an invariant (immutable posted entry, append-only audit) | You bypassed the engine or the operation is illegal — never "fix" this with raw SQL |
| `EXPORT_EMPTY_YEAR` | No posted entries in the year | Book something first, or pick a different year — drafts are never exported |
| `INVALID_DATE` / `INVALID_YEAR` / `INVALID_PERIOD` / `INVALID_WINDOW` / `INVALID_LIMIT` / `INVALID_DUE_DAYS` | A date, year, period, day-window, row-limit or due-days argument is malformed (e.g. `2026-13`, `2026-02-30`, `--limit abc`, `--due-days abc`) | Use the format the flag help shows — these are rejected before anything is written, so nothing is half-booked. `--due-days 0` means due on the invoice date (kept as 0, never silently defaulted) |
| `INVALID_STATE` | A bank transaction state other than `unmatched`/`matched`/`ignored` was requested | Pass one of the three valid states — a typo'd state would corrupt the reconciliation flow |
| `ACTOR_REQUIRED` / `INVALID_ACTOR` | `--actor` missing or not `'<role>:<name>'` (bare `human`/`agent` rejected) | Always pass `--actor '<role>:<name>'` or `BUKIO_ACTOR` |
| `ALREADY_ENROLLED` / `INVALID_KEY` / `NOT_ENROLLED` / `INVALID_REASON` | Actor-key registry: key already active, missing keyid/public key, actor not enrolled, or revocation without a reason | `actor register` a fresh key after `actor revoke` (rotation); always pass a revocation reason |
| `KEY_NOT_FOUND` / `KEY_ALREADY_EXISTS` / `PASSPHRASE_REQUIRED` / `PASSPHRASE_INVALID` / `UNLOCK_NOT_APPLICABLE` / `INVALID_TTL` / `INVALID_ENFORCE` | Actor key/session CLI: missing key file, keygen overwrite without `--force`, human passphrase missing or wrong, unlock for a non-human key, bad `--ttl-hours`, or `enforce` without exactly one of `--on`/`--off` | `actor keygen` first; `--force` to rotate; set `BUKIO_SIGNING_PASSPHRASE` or run interactively; pass `--on` or `--off` |
| `SIGNATURE_REQUIRED` / `SIGNATURE_INVALID` / `SIGNATURE_STALE` / `NONCE_REUSED` / `ACTOR_KEY_UNKNOWN` / `ACTOR_KEY_REVOKED` | Sign gate (only when the company enforces signing): no key material for the actor, signature does not verify (key mismatch/tampered), timestamp outside the ±5 min window, nonce replayed (refused in every mode), actor not enrolled, or enrolled but revoked | Enrol the actor (`actor register`), rotate a revoked/mismatched key (`actor keygen --force` + `register`), unlock human keys (`actor unlock` / `BUKIO_SIGNING_PASSPHRASE`); `actor enforce --off` is the recovery escape hatch |
| `AUTHZ_DENIED` | Tier 0.5 gate (only when `actor authz --on`): the signed actor lacks the capability the command needs (or the command has no capability mapping — fail closed), or `actor revoke --target` without the owner role (owner-mediated kill, required regardless of authz mode) | `actor roles` to see your roles, `actor can '<cmd>'` to check a command, ask the owner to grant a role (`actor roles grant <role> --for <who>`) |
| `INVALID_AUTHZ` | `actor authz` without exactly one of `--on`/`--off` | Pass exactly one |
| `INVALID_ROLE` / `ROLE_NOT_GRANTED` / `LAST_OWNER` | Role registry: unknown role, revoking a role the actor does not hold, or revoking the LAST owner (a company needs at least one owner — to turn authz off and to mediate key revokes) | Use one of `owner\|bookkeeper\|payments\|tax\|assets\|readonly`; grant the role first; grant `owner` to another actor before stepping down |
| `INVALID_DESCRIPTION` / `INVALID_POSTINGS` / `INVALID_AMOUNT_CENTS` | Description empty, posting spec malformed, or a posting amount is 0 | Fix the argument — every entry needs a description, ≥2 non-zero postings |
| `INVALID_LEGAL_FORM` / `INVALID_VAT_CHOICE` / `INVALID_FISCAL_YEAR_END` | `init` got a bad legal form, `--vat` other than `on`/`off`, or an impossible `--fiscal-year-end` (e.g. `99-99`, `02-30`) | Use the values the flag help shows (calendar dates only) |
| `INVALID_COUNTRY` / `COUNTRY_NOT_SUPPORTED` / `PROFILE_NOT_FOUND` / `COUNTRY_IMMUTABLE` | `init --country` / jurisdiction-profile resolution: not an ISO 3166-1 alpha-2 code, a valid country with no profile implemented yet (Phase A supports NL; GB/US/FR/LU planned), no profile for a valid code, or `company update --country` trying to change the country after init (one company = one country) | Use a 2-letter code; supported: NL; re-init a new DB for another country |
| `INVALID_IBAN` / `INVALID_NAME` / `INVALID_TYPE` / `NOTHING_TO_UPDATE` | Bad company/contact field on `init`/`company update`/`contact add` (IBAN mod-97 validated) | Fix the value; pass at least one change to `company update` |
| `COMPANY_REQUIRED` / `COMPANY_INCOMPLETE` / `NOT_INITIALISED` | Company missing or incomplete (e.g. no valid company IBAN for SEPA) | Run `init`, then `company update` to complete the profile |
| `PAYMENT_FORMAT_NOT_SUPPORTED` | SEPA export (pain.001/pain.008) hit a format the jurisdiction profile does not declare in exchange.paymentFormats | The declared formats come from the profile; Phase A (NL) supports sepa-pain.001 + sepa-pain.008 |
| `DEADLINE_RULE_NOT_FOUND` / `INVALID_PERIOD_SHAPE` | compliance calendar hit a filing type whose deadlineRule or periodShape has no implementation (Phase A: nl-quarterly, nl-13-months; YYYY-Qn, YYYY) | Filing types come from the jurisdiction profile; add the rule/shape or fix the profile |
| `INVALID_CURRENCY` / `INVALID_RATE` / `INVALID_FX_AMOUNT` / `INVALID_FX_CURRENCY` / `FX_RATE_NOT_FOUND` | FX rate or foreign-currency posting is malformed, or no rate exists | `fx set` a rate, pass `--rate`, or allow the ECB fetch |
| `ECB_FETCH_FAILED` / `ECB_RATE_NOT_AVAILABLE` | ECB unreachable, or the currency/date has no reference rate | Retry later or `fx set` a rate manually |
| `INVALID_FREQUENCY` / `INVALID_RUNS` / `INVALID_RANGE` / `INVALID_REVERSE` | Recurring template arguments malformed (frequency, run count, date range, reverse flag) | Fix the template arguments |
| `INVALID_SCHEMA` / `INVALID_COST` / `INVALID_LIFE` / `INVALID_RESIDUAL` / `ASSET_NOT_FOUND` | Assets arguments malformed (scheme, cost, life months, residual value) or asset id unknown | Fix the arguments |
| `INVALID_DAY` | Recurring/period day-of-period not an integer 1–28 (29–31 would clamp into the next month) | Use a day between 1 and 28 |
| `VAT_MODULE_OFF` / `VAT_CODE_NOT_FOUND` / `INVALID_VAT_AMOUNT` / `VAT_MARGIN_NOT_SUPPORTED` / `KOR_ACTIVE` | VAT module off / unknown `@CODE` / bad VAT amount / margin on an unsupported line / KOR blocks `vat book` | `vat enable` first; use the codes `vat enable` creates; `@M` is unsupported for some paths |
| `VAT_NOTHING_TO_FILE` / `VAT_SETTLE_NOTHING` | `vat file`: 1500/2500 net is zero (nothing to reclassify) / `vat settle`: the af-te-dragen account balance is zero (nothing filed, already settled, or the wrong `--account`) | Check the VAT readout + `report trial-balance` for 1500/2500/2510 before filing/settling; pass the account `vat file` reported |
| `VAT_ACCOUNT_COLLISION` | `vat file`: the requested `--account` exists but is NOT the 'Af te dragen omzetbelasting' account and has no free numeric successor (non-numeric code, or a run of 1000 taken codes) | The engine auto-falls to the next free numeric code (2510 → 2511 → …) and reports it; only when that is impossible, pick a free code with `--account` |
| `VAT_SETTLE_DIRECTION` | `vat settle`: the bank transaction's direction doesn't match the balance (outgoing needed for te betalen, incoming for a refund) | Pick the correct bank transaction |
| `VAT_SETTLE_DIFFERENCE_TOO_LARGE` | `vat settle`: |payment − af te dragen balance| > €5 — a VAT filing rounds per line (< €0.50/line), so this is the wrong amount | Double-check the payment transaction and the filed period |
| `INVALID_DIFFERENCE_ACCOUNT` | `vat settle`: the `--difference-account` code does not exist | Use an existing P&L account (default 4700) or `account add` it first |
| `NOT_FINALIZED` / `CONTACT_NOT_FOUND` / `INVOICE_REF_REQUIRED` / `PDF_UNAVAILABLE` | Invoice not finalized, unknown contact, missing invoice reference, PDF engine unavailable | Finalize first; fix the contact; `PDF_UNAVAILABLE` means headless Chromium isn't installed |
| `PEPPOL_NOT_CONFIGURED` / `PEPPOL_SEND_FAILED` / `PEPPOL_BUYER_MISSING_ID` / `PEPPOL_BUYER_REFERENCE_MISSING` | No Peppol endpoint, the provider rejected/unreachable, the buyer has no KVK number (BT-49 requires it), or the invoice has no buyer reference (BT-10, PEPPOL-EN16931-R003 / DE-R-015) | Set `BUKIO_PEPPOL_ENDPOINT`/`BUKIO_PEPPOL_TOKEN`; check the provider response; set the buyer KVK via `contact update --id <id> --kvk <number>`; set the reference at `invoice create ... --reference <text>` |
| `SMTP_NOT_CONFIGURED` / `SMTP_CONNECT_FAILED` / `SMTP_AUTH_FAILED` / `SMTP_SEND_FAILED` / `CONTACT_EMAIL_MISSING` | `invoice email`: SMTP env vars missing, server unreachable/TLS failed, auth rejected, send rejected, or the contact has no email (and no `--to`) | Set `BUKIO_SMTP_HOST`/`PORT`/`USER`/`PASS`/`FROM`; add the contact email or pass `--to` |
| `UPDATE_NOT_A_CLONE` / `UPDATE_WRONG_REMOTE` / `UPDATE_NO_REMOTE_BRANCH` / `UPDATE_CONFIRM_REQUIRED` / `UPDATE_GIT_FAILED` | `bukio update`: path is not a git clone (npm installs need `npm update -g`) / origin is not the official repo (forks: `--trust-remote`) / no `origin/main` after fetch / local changes present but no `--yes` / a git command failed | Run `--dry-run` first, add `--yes` to confirm the overwrite, `--trust-remote` on a fork, or fix the git failure |
| `INVALID_CAMT` / `INVALID_CSV_HEADER` / `EMPTY_STATEMENT` / `ALREADY_MATCHED` | Bank file unparsable, bad header, empty statement, or a matched transaction re-linked | Use a real CAMT.053/CSV export; unmatch first |
| `MANDATE_REQUIRED` / `MANDATE_NOT_FOUND` / `MANDATE_DUPLICATE` / `INVALID_MANDATE_REF` / `INVALID_SCHEME` / `PAYABLE_NOT_DIRECT_DEBIT` | Direct-debit batch: contact has no mandate (or it was deleted), duplicate/oversized/empty ref, scheme not core\|b2b, or a transfer payable in an incasso batch | `payments mandate add --contact N --ref R [--type b2b]` first; keep refs ≤ 35 chars; match the payable's payment term to the batch kind |
| `INVALID_XAF` / `EMPTY_CSV` / `IMPORT_VALIDATION_FAILED` / `FILE_NOT_FOUND` | Import file unreadable/empty/structurally invalid, or validation failed with per-line details | The error lists every offending line — fix and re-run |
| `INVALID_UBL_INVOICE` / `UNSUPPORTED_UBL_DOCUMENT` | Inbound e-invoice not an EN 16931 `<Invoice>` (or missing ID/dates/amount), or a document type bukio won't import (credit notes 381 — payables are positive-only) | Import a real 380 invoice UBL; credit notes are booked manually for now |
| `INCOMPLETE_YEAR` / `ALREADY_CLOSED` / `OUT_REQUIRED` | Year-end close blocked: drafts exist, or the year is already closed, or `--out` is required | Clear drafts / pick another year / pass `--out` |
| `INVALID_FORMAT` / `INVALID_STATUS` / `INVALID_VALUE` / `INVALID_SOURCE` / `INVALID_KIND` | Report format, status filter, value, source or kind argument malformed | Use the values the flag help shows |
| `BATCH_VALIDATION_FAILED` / `BATCH_NOT_FOUND` / `BATCH_ALREADY_EXPORTED` / `EMPTY_BATCH` / `SEPA_NAME_TOO_LONG` | SEPA batch invalid (per-line details), unknown, already exported once, empty, or a beneficiary/company name exceeds 70 chars (SEPA Max70Text) | The error lists every bad line; fix and re-create; keep names ≤ 70 chars |
| `PAYABLE_NOT_FOUND` / `PAYABLE_NOT_ELIGIBLE` / `ALREADY_PAID` / `MSGID_TOO_LONG` / `INVALID_METHOD` / `PAYABLE_DUPLICATE` | Payable unknown/not transferable/already paid, MsgId > 35 chars, bad payment method, or an unpaid payable with the same (contact, invoice_ref) already exists | Use `payments payables list`; payables are single-use — settle or remove the earlier payable before re-adding the same invoice |
| `INVALID_BACKUP` / `RESTORE_EXISTS` / `SAME_FILE` | Backup file invalid, restoring over an existing company (needs `--force`), or backup = restore path | Use a real backup file; pass `--force` deliberately |
| `BACKUP_PASSPHRASE_REQUIRED` / `BACKUP_PASSPHRASE_WRONG` / `INVALID_KEEP` | `backup --encrypt` / encrypted restore without a passphrase, wrong passphrase (or corrupted file), or `--keep` not a positive integer / combined with `--out` | Pass `--passphrase` or set `BUKIO_BACKUP_PASSPHRASE`; rotation applies to the default backup folder only |
| `MCP_READONLY` | `BUKIO_MCP_READONLY=1` blocked a mutation | Plan-only in that mode; unset the env to execute |
| `LOCAL_ONLY` | `--server` was set for a command that cannot run remotely (`server start`, `server token`, `mcp`, `init`, `update`, `actor keygen/unlock/lock`) | Run it on the machine where it belongs (the server/operator machine for `server *`, the client for key material) |
| `REMOTE_UNREACHABLE` / `REMOTE_ERROR` | `--server <url>` could not be reached (connection refused, DNS, TLS) or the server returned a non-JSON/invalid response | Check the URL, that `bukio server start` is running, and the network path (Tailscale/TLS) |
| `TOKEN_REQUIRED` / `TOKEN_INVALID` / `TOKEN_EXPIRED` / `TOKEN_USED` / `TOKEN_ACTOR_MISMATCH` | Remote enrolment: `--token` missing with `--server`, unknown token, TTL expired, already redeemed (single-use), or minted for a different actor | Mint a fresh token with `bukio server token <actor>` on the server machine and redeem it once within the TTL |
| `BAD_JSON` / `BODY_TOO_LARGE` / `INVALID_ENVELOPE` / `CMD_MISMATCH` / `INVALID_LISTEN` / `TLS_KEY_REQUIRED` | Server rejected the request: body not JSON, > 10 MB, envelope missing `args.argv`, `cmd` does not match the argv's first words, `--listen` not `host:port`, or `--tls-cert` without `--tls-key` | Use the official CLI as the client; fix the `--listen`/TLS flags on `server start` |
| `SERVER_EXEC` | The server could not spawn its own CLI (`node bin/bukio.js`) to run the verified envelope | Check the node runtime on the server host |
| `ACCOUNT_EXISTS` / `ACCOUNT_TYPE` / `INVALID_CODE` / `INVALID_NORMAL_BALANCE` / `INVALID_RGS_CODE` | Account code taken, bad type, or malformed code/balance/rgs on `account add`/`import` | Fix the account arguments (RGS like `WKPR.70`) |
| `ALREADY_ACTIVE` / `ALREADY_INACTIVE` / `ALREADY_COMPLETED` / `ALREADY_DISPOSED` / `SCHEME_NAME_TAKEN` / `SCHEME_NOT_FOUND` | Assets state conflicts (activate/pause/dispose/completed) or scheme name taken/unknown | Check `assets list` / `assets schemes list` first |
| `INVALID_COMBINATION` / `INVALID_DEPRECIATION` / `INVALID_LINE` / `INVALID_MODEL` / `INVALID_RESIDUAL` | Assets/jaarrekening argument combos malformed (depreciation method, line spec, micro/klein model) | Use the values the flag help shows |
| `ALREADY_FINALIZED` / `CREDIT_NOT_PAYABLE` / `NOT_PAYABLE` / `NOT_SALES_INVOICE` / `NO_LINES` / `OVERPAYMENT` / `ENTRY_NOT_FOUND` / `CUSTOMER_INCOMPLETE` / `CUSTOMER_VAT_REQUIRED` / `SUPPLIER_INCOMPLETE` | Invoice lifecycle guard: already finalized, credit of a non-sales invoice, pay on a non-payable invoice, missing lines, overpayment, unknown entry, or missing customer/supplier fields (12-vereisten) | Complete the customer/supplier profile (`contact add` with address), then finalize |
| `FX_DIFFERENCE_TOO_LARGE` | FX-difference leg exceeds the tolerance at booking | Re-check the rate — or book the difference explicitly |
| `ICP_VAT_ID_MISSING` / `NO_COMPANY` | ICP readout needs customer btw-ids; company row missing | Add btw-ids to EU customers; run `init` first |
| `ITEM_NOT_FOUND` / `ITEM_INACTIVE` / `INVALID_ITEM_SPEC` / `INVALID_ITEM_OVERRIDE` / `INVALID_UNIT` / `INVALID_PRICE` / `INVALID_VAT_CODE` | Items catalog: unknown/deactivated item, malformed `--items` spec, bad price, quantity-unit or VAT-code override | `item list` to find ids; fix the spec (`ID[:QTY][@PRICE][@VATCODE][@-DISCOUNT]`); units are `h|day|month|unit|session|km|kg|project` |
| `INVALID_LINE_DISCOUNT` / `INVALID_DISCOUNT` / `INVALID_LANGUAGE` / `CONFLICTING_LINES` | Discount outside (0,100]% or ≥ line amount, both `--discount-pct` and `--discount-amount` passed, language not `nl`/`en`, or both `--lines` and `--items` passed | Fix the arguments — exactly one line source and one discount type; discounts apply before VAT |
| `LOGO_FILE_NOT_FOUND` / `LOGO_TOO_LARGE` / `LOGO_DIMENSIONS_TOO_LARGE` / `LOGO_UNSUPPORTED_FORMAT` / `LOGO_NOT_SET` | Logo file missing, > 1 MB, > 2048×2048 px, not PNG/JPEG/SVG, or `company logo` with no stored logo | Use a PNG/JPEG/SVG ≤ 1 MB (≤ 2048×2048 px); set it with `company update --logo` |
| `ATTACHMENT_FILE_NOT_FOUND` / `ATTACHMENT_TOO_LARGE` / `ATTACHMENT_EMPTY` / `ATTACHMENT_NOT_FOUND` / `ATTACHMENT_DUPLICATE` / `ATTACHMENT_FILE_MISSING` / `REF_REQUIRED` / `FILE_EXISTS` / `INVALID_STORE` | `attach` problems: file missing, > 25 MB, empty file, unknown id, exact duplicate, file-mode copy missing on disk, neither/`--entry` nor `--invoice` given, `--out` target exists (needs `--force`), or `--store` not `db`/`file` | Use `attach list` to find ids; pass exactly one ref; pass `--force` to overwrite; `--store db` (default, travels with backups) or `file` |

> Every code above is emitted by at least one command — the docs are kept in sync by `test/hardening.test.js` (all codes in `src/` must appear in this table).

---

## 8. Anti-patterns (never do these)

- ❌ Posting without `--dry-run` first.
- ❌ Omitting `--actor '<role>:<name>'` — the CLI rejects it, and the audit log must always name who acted.
- ❌ Editing/deleting rows with sqlite3 or raw SQL — the triggers will (rightly) reject it, and the audit log must stay truthful.
- ❌ "Correcting" a posted entry by editing its postings — postings of non-draft entries are immutable by design. Use `reverse`.
- ❌ Using floats or human-formatted strings in calculations — always `amount_cents`.
- ❌ Reusing one database for two companies.
- ❌ Assuming VAT works — it doesn't yet (Phase 2). The core ledger is deliberately VAT-free.
- ❌ Ignoring `balanced: false` — investigate before continuing; the books must reconcile.

---

## 9. Capability boundaries (Phase 9 complete)

Available: company init (incl. address for compliant invoices), journal entries (incl. **FX conversion**), accounts, reports (trial balance, balance sheet, P&L, journal — JSON/CSV/XLSX), bank (CAMT.053 + Dutch CSV import, idempotent hashing, auto-match/link/post reconciliation incl. invoice payments), optional VAT module (enable, `vat book`, OB readout 1a–5d + mark-filed), recurring entries + invoices, invoicing (lifecycle, 12-vereisten, PDF, UBL/Peppol BIS 3.0, credit notes, payments), Peppol send, jaarrekening (`year-end close`, micro/klein statutory accounts, KVK deposit PDF, XLSX), ICP readout, **FX rates + foreign-currency booking** (`fx set`, `--currency/--rate` on entry add + vat book, `fx_currency`/`fx_amount_cents` on postings), **MCP server** (`bukio mcp`, plan-only mutations, READONLY mode), **compliance calendar** (`compliance status`/`mark`), **imports** (`import opening-balances` / `journal` / `xaf` — whole-file validation, idempotent), **month-end close check** (`month-end --period`), **invoice reminders** (`invoice reminders`, draft emails only — never sends), **fixed assets** (`assets scheme add`/`assets add`/`run`/`register`/`dispose`/`pause`/`resume` — mid-life adoption via recognition-date + cum-dep at recognition, lineair + degressief, activastaat, source 'assets'), **SEPA payment batches** (`payments payables add`/`list`/`pay` — transfer vs direct-debit; `payments batch create`/`export`/`list`/`show`/`delete` — pain.001 `001.03`/`001.09`, unique MsgId, one export per batch, contact IBANs mod-97 validated), **audit-file export** (`export xaf` — Auditfile Financieel 4.0 XML for external advisors; audit log as csv/xlsx), backup/restore, audit log, `--json`/`--dry-run` everywhere, actor attribution, per-company databases.

**Not yet available:** LLM categorization suggestions, Ponto live feeds, Peppol receive, OCR, automated email sending for reminders. Do not pretend these exist; propose them to the maintainer instead of fabricating workarounds. NL query is done via the MCP tools — there is no bespoke NL parser.
