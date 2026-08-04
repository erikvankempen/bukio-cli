# bukio-cli

**Agent-first double-entry bookkeeping for Dutch SMEs.** Runs natively on a VPS, stores everything in a local SQLite database, and is designed so AI agents — not just humans — can operate it safely and auditably.

- **Agent-native** — every command emits deterministic `--json`; every mutation supports `--dry-run` (plan mode); every action lands in an append-only audit log.
- **VAT optional** — the core ledger is VAT-agnostic. A KOR / non-VAT entity never touches VAT concepts. The VAT module (Phase 2) adds codes, returns and a manual-filing readout when enabled.
- **Single company per database** — a second company = a second database (flag `--db` or env `BUKIO_DB`).
- **No automated tax filing** — bukio-cli never submits anything to the Belastingdienst. It computes what you need and you file manually.
- **Local-first** — no cloud dependency, no lock-in. Your 7-year administration stays yours.

---

## Table of Contents

1. [Status](#status)
2. [Requirements & Install](#requirements--install)
3. [Core Concepts](#core-concepts)
4. [Quickstart](#quickstart)
5. [Command Reference](#command-reference)
6. [Global Flags](#global-flags)
7. [Money Format](#money-format)
8. [Integrity & Safety Model](#integrity--safety-model)
9. [The Database](#the-database)
10. [Using Agents](#using-agents)
11. [Project Layout](#project-layout)
12. [Development & Testing](#development--testing)
13. [Error Codes](#error-codes)
14. [Common Tasks](#common-tasks)
15. [Roadmap](#roadmap)

---

## Status

**Phase 2 — Bank & VAT module (current).** Phases 0–1 (ledger, accounts, reports, backup) plus Phase 2 (bank import + matching, optional VAT module with OB readout) are complete. See [Roadmap](#roadmap) for what comes next.

---

## Requirements & Install

- Node.js **>= 20**
- Linux/macOS (developed on a Linux VPS)

```bash
git clone https://github.com/erikvankempen/bukio-cli.git
cd bukio-cli
npm install          # deps: better-sqlite3, commander
npm link             # exposes `bukio` on PATH (or: npm install -g .)
bukio --version
```

Uninstall: `npm unlink -g bukio-cli` (or `npm uninstall -g bukio-cli`).

---

## Core Concepts

### Double-entry bookkeeping

Every **journal entry** contains two or more **postings** (debits and credits) whose amounts sum to **zero**. Positive amounts are debits, negative amounts are credits. This invariant is enforced by the engine at creation time *and* by a database trigger when an entry is posted — an unbalanced posted entry is impossible.

### Accounts and the chart of accounts

Accounts are organised in a **chart of accounts** with 4-digit codes and a type:

| Type | Normal balance | Examples |
|------|----------------|----------|
| `asset` | debit | 1000 Kas, 1100 Bank, 1200 Debiteuren |
| `liability` | credit | 2000 Crediteuren, 2100 Overige schulden |
| `equity` | credit | 3000 Eigen vermogen |
| `expense` | debit | 4000 Inkoopwaarde, 4100–4500 kosten |
| `income` | credit | 8000 Omzet, 8100 Overige opbrengsten |

`bukio init` seeds a minimal default chart (14 accounts, no VAT accounts — the core is VAT-agnostic). The full RGS (Referentie Grootboekschema) taxonomy import arrives in Phase 1; account codes are RGS-compatible in structure.

### Entry lifecycle

```
draft ──post──▶ posted ──reverse──▶ (original stays posted)
  │                                  + contra-entry posted (negated postings)
  └──reverse── (not allowed)          + audit trail
```

- **draft** — a work-in-progress entry. Postings can be added/changed/removed (via SQL or future commands). Drafts are excluded from reports.
- **posted** — final. Postings are immutable (database trigger). Posted entries appear in the trial balance.
- **reverse** — reversing a posted entry **posts a linked contra-entry** with negated postings. The original entry *stays posted* — the contra-entry cancels it, so the net effect on the books is zero. Linkage: the contra-entry's `reversed_from_id` points at the original; the audit log records the action. Posted entries are **never deleted** — they are reversed.

### Actors

Every mutation records an **actor** — `human` by default, or `agent:<name>` when an agent acts (e.g. `--actor agent:hermes`). Actors appear on entries (`created_by`) and in the audit log, so a human can always see exactly what an agent did.

### The audit log

An **append-only** log of every mutation: actor, action, command, JSON args, outcome, and affected entry IDs. Database triggers block UPDATE and DELETE — the log cannot be rewritten after the fact. Read it with `bukio audit`.

### Amounts

All money is stored as **integer cents** (`amount_cents`). There are no floats anywhere in financial code paths. See [Money Format](#money-format).

---

## Quickstart

```bash
# 1. Initialise a company (dry-run first — it writes nothing)
bukio init --name "Demo BV" --kvk 12345678 --legal-form bv --vat on --dry-run

# 2. Actually initialise (creates ~/.bukio/bukio.db + 14-account chart)
bukio init --name "Demo BV" --kvk 12345678 --legal-form bv --vat on

# 3. Post the opening capital (positive = debit, negative = credit)
bukio entry add --date 2026-08-04 --desc "Startkapitaal" \
  --postings "1100:10000.00,3000:-10000.00" --post

# 4. Book an expense
bukio entry add --date 2026-08-05 --desc "Kantoorartikelen" \
  --postings "4300:250.00,1100:-250.00" --post

# 5. Check the books balance
bukio report trial-balance

# 6. See who did what
bukio audit
```

---

## Command Reference

Global flags (`--json`, `--db`, `--actor`) can appear before or after the subcommand. See [Global Flags](#global-flags).

### `bukio init`

Initialise a company database: creates the file, the company row, and seeds the default chart of accounts.

| Option | Default | Description |
|--------|---------|-------------|
| `--name <name>` | *(required)* | Company name |
| `--kvk <kvk>` | — | KVK number |
| `--legal-form <form>` | `eenmanszaak` | `eenmanszaak` \| `vof` \| `bv` \| `nv` \| `stichting` \| `vereniging` |
| `--btw-id <id>` | — | BTW identification number |
| `--iban <iban>` | — | Bank account (IBAN) |
| `--vat <on\|off>` | `off` | Enable the VAT module (Phase 2) |
| `--kor` | off | Small business scheme — implies `--vat off` |
| `--fiscal-year-end <mm-dd>` | `12-31` | Fiscal year end |
| `--dry-run` | off | Show the plan without writing anything |

Fails with `ALREADY_INITIALISED` if the database already has a company.

```bash
bukio init --name "Demo BV" --kvk 12345678 --legal-form bv --vat on --dry-run
bukio init --name "Demo BV" --kvk 12345678 --legal-form bv --vat on
```

### `bukio entry add`

Create a journal entry (draft by default; `--post` posts it immediately).

| Option | Default | Description |
|--------|---------|-------------|
| `--date <yyyy-mm-dd>` | today | Entry date (ISO) |
| `--desc <description>` | *(required)* | Description |
| `--postings <CODE:AMOUNT>` | *(required)* | Posting spec — **repeat the flag or comma-separate**; positive = debit, negative = credit |
| `--source <source>` | `manual` | `manual` \| `bank` \| `invoice` \| `agent` |
| `--source-ref <ref>` | — | Source reference (e.g. invoice number) |
| `--post` | off | Post immediately (draft → posted) |
| `--dry-run` | off | Validate and show the plan without writing |

```bash
# two postings, comma-separated
bukio entry add --date 2026-08-04 --desc "Startkapitaal" \
  --postings "1100:10000.00,3000:-10000.00" --post

# equivalent: repeated flag
bukio entry add --desc "Startkapitaal" \
  --postings "1100:10000.00" --postings "3000:-10000.00"

# three postings (VAT-like split is a Phase 2 concern; 3-leg entries work today)
bukio entry add --desc "3-leg example" \
  --postings "1100:121.00,8000:-100.00,2100:-21.00" --dry-run
```

Validation errors (see [Error Codes](#error-codes)): `INVALID_POSTING`, `INVALID_AMOUNT`, `INVALID_DATE`, `INVALID_DESCRIPTION`, `TOO_FEW_POSTINGS`, `UNBALANCED`, `ACCOUNT_NOT_FOUND`, `ACCOUNT_INACTIVE`, `INVALID_AMOUNT_CENTS`, `INVALID_SOURCE`.

### `bukio entry post`

Post a draft entry (draft → posted).

| Option | Default | Description |
|--------|---------|-------------|
| `--id <id>` | *(required)* | Entry id |
| `--dry-run` | off | Show the plan without writing |

The database trigger backstops the invariant: an entry needs **>= 2 postings summing to zero** before it can be posted.

### `bukio entry reverse`

Reverse a posted entry: **posts a linked contra-entry** with negated postings. The original stays posted; the contra-entry cancels it (net effect zero). See [Core Concepts](#core-concepts).

| Option | Default | Description |
|--------|---------|-------------|
| `--id <id>` | *(required)* | Entry id |
| `--reason <text>` | — | Reason, appended to the contra-entry description |
| `--dry-run` | off | Show the planned contra-entry without writing |

Fails with `NOT_POSTED` for drafts and `ALREADY_REVERSED` if a posted reversal already exists.

```bash
bukio entry reverse --id 2 --reason "verkeerde categorie" --dry-run
bukio entry reverse --id 2 --reason "verkeerde categorie"
```

### `bukio entry list`

List journal entries (newest first).

| Option | Default | Description |
|--------|---------|-------------|
| `--state <state>` | all | `draft` \| `posted` \| `reversed` |
| `--date-from <yyyy-mm-dd>` | — | Earliest date (inclusive) |
| `--date-to <yyyy-mm-dd>` | — | Latest date (inclusive) |
| `--limit <n>` | `100` | Max rows |

### `bukio entry show`

Show one entry with its full postings.

| Option | Default | Description |
|--------|---------|-------------|
| `--id <id>` | *(required)* | Entry id |

### `bukio report trial-balance`

Per-account debit/credit/net totals from **posted** entries, with a final BALANCED/UNBALANCED verdict. Drafts and the mirror of reversed entries behave per the lifecycle rules (drafts excluded; contra-entries included — that's what makes reversals net to zero).

| Option | Default | Description |
|--------|---------|-------------|
| `--year <yyyy>` | all years | Filter by year |
| `--format <format>` | human (json with `--json`) | `json` \| `csv` \| `xlsx` \| `human` |
| `--out <path>` | stdout | Output file (required for xlsx) |

### `bukio report balans`

Balance sheet as of a date, grouped by RGS hoofdgroep (Materiële vaste activa, Voorraden, Vorderingen, Liquide middelen / Eigen vermogen, Kortlopende schulden, …). Includes the computed **Nog te verdelen resultaat** (net result of income/expense accounts). Invariant: **total assets = total liabilities + equity + result** — the report says `BALANCED` or `UNBALANCED!`.

| Option | Default | Description |
|--------|---------|-------------|
| `--as-of <yyyy-mm-dd>` | today | Balance date (inclusive) |
| `--format <format>` | human (json with `--json`) | `json` \| `csv` \| `xlsx` \| `human` |
| `--out <path>` | stdout | Output file (required for xlsx) |

### `bukio report pnl`

Winst-en-verliesrekening for a period, grouped by RGS hoofdgroep (Omzet, Inkoopwaarde van de omzet, Personeelskosten, Afschrijvingen, Overige bedrijfskosten, Financiële baten en lasten, …). Reports revenue, costs and **Netto resultaat**.

| Option | Default | Description |
|--------|---------|-------------|
| `--year <yyyy>` | current year | Fiscal year (sets from/to) |
| `--from <yyyy-mm-dd>` | year start | Period start (inclusive) |
| `--to <yyyy-mm-dd>` | year end | Period end (inclusive) |
| `--format <format>` | human (json with `--json`) | `json` \| `csv` \| `xlsx` \| `human` |
| `--out <path>` | stdout | Output file (required for xlsx) |

### `bukio report journal`

Journal export — one row per posting with account info, for a period. Ideal for handing to your boekhouder.

| Option | Default | Description |
|--------|---------|-------------|
| `--year <yyyy>` | current year | Fiscal year (sets from/to) |
| `--from <yyyy-mm-dd>` | year start | Period start (inclusive) |
| `--to <yyyy-mm-dd>` | year end | Period end (inclusive) |
| `--format <format>` | human (json with `--json`) | `json` \| `csv` \| `xlsx` \| `human` |
| `--out <path>` | stdout | Output file (required for xlsx) |

```bash
bukio report balans --as-of 2026-12-31
bukio report pnl --year 2026 --format xlsx --out ~/exports/pnl-2026.xlsx
bukio report journal --year 2026 --format csv --out ~/exports/journal-2026.csv
```

### `bukio account`

Chart of accounts management.

| Command | Purpose |
|---------|---------|
| `account add --code <c> --name <n> --type <t> --normal-balance <d\|c> [--rgs-code <r>] [--dry-run]` | Add an account |
| `account list [--type <t>] [--include-inactive]` | List accounts |
| `account show --code <c>` | Show one account |
| `account deactivate --code <c>` | Deactivate (blocks new postings; history stays) |
| `account reactivate --code <c>` | Reactivate |
| `account import --file <chart.csv> [--dry-run]` | Import a chart from CSV: `code,name,type,normal_balance[,rgs_code]` |

The bundled default chart lives at `assets/chart-nl.csv` — you can import it (or your own) into any database:

```bash
bukio account import --file assets/chart-nl.csv --dry-run   # validate first
bukio account import --file assets/chart-nl.csv
```

### `bukio bank`

Bank accounts, import and matching.

| Command | Purpose |
|---------|---------|
| `bank add --iban <IBAN> [--name] [--account-code 1100]` | Register a bank account (links to a ledger account) |
| `bank list` | Accounts with balance, transaction and unmatched counts |
| `bank import --file <path> --iban <IBAN> [--format camt\|csv\|auto] [--dry-run]` | Import transactions — **CAMT.053 XML** or bank CSV (Rabo/ING/ABN column aliases, Dutch `1.234,56` amounts, Af/Bij sign). Idempotent via SHA-256 hash. |
| `bank transactions [--iban] [--state unmatched\|matched\|ignored] [--limit]` | List transactions |
| `bank match auto [--window-days 5] [--dry-run]` | Auto-match unmatched transactions to posted entries (exact ≤ 2 days, fuzzy ≤ window) |
| `bank match suggest` | Unmatched transactions with a proposed posting (income → 8000, expense → 4300) |
| `bank match link --tx <id> --entry <id> [--method]` | Link a transaction to an existing posted entry |
| `bank match post --tx <id> --account <code> [--dry-run]` | Post a new entry from an unmatched transaction (bank leg + counter leg), reconciled automatically |
| `bank ignore --tx <id>` / `bank unignore --tx <id>` | Ignore/re-open a transaction (e.g. transfers between own accounts) |

The **bank balance vs ledger balance check** is the reconciliation test: after matching everything, `bank list` balance should equal the ledger account balance in the trial balance.

### `bukio vat`

Optional VAT module (per company; KOR companies cannot enable it).

| Command | Purpose |
|---------|---------|
| `vat enable [--dry-run]` | Enable the module: accounts 1500 (te vorderen) + 2500 (te betalen), 8 VAT codes |
| `vat codes` | List VAT codes (21, 9, 0, V vrijgesteld, R/RE verlegd, M marge, P privé) |
| `vat book --date --desc --postings "1100:121.00,8000:-100.00@21" [--post] [--dry-run]` | Book a VAT-aware entry. `@CODE` tags a posting as net; the VAT amount and the VAT ledger leg (2500/1500) are computed automatically. |
| `vat readout --period 2026-Q2 [--mark-filed]` | **OB-aangifte manual-filing readout** — fields 1a–5d for the period (quarter `YYYY-Qn` or month `YYYY-MM`). bukio never files; you enter these amounts in Mijn Belastingdienst Zakelijk. `--mark-filed` records the filing. |

```bash
# sale: 121.00 incl 21% -> omzet 100 + te betalen btw 21
bukio vat book --date 2026-06-01 --desc "Factuur 2026-001" \
  --postings "1100:121.00,8000:-100.00@21" --post

# purchase: 60.50 incl 21% -> kosten 50 + te vorderen btw 10.50
bukio vat book --date 2026-06-05 --desc "Kantoorartikelen" \
  --postings "4300:50.00@21,1100:-60.50" --post

# quarterly manual filing aid
bukio vat readout --period 2026-Q2
```

**OB field mapping:** 1a/1b/1c omzet (21%/9%/0%/vrijgesteld), 1d privégebruik, 3a/3b/3c inkopen, 4a/4b verlegde btw (binnenland/EU, netted via 5b), 5a verschuldigde btw, 5b voorbelasting, 5d te betalen/te ontvangen. Fields 2a/2b (exports) and 5c are not tracked in Phase 2 (shown as 0).

### `bukio backup` / `bukio restore`

| Command | Purpose |
|---------|---------|
| `backup [--out <path>]` | Consistent SQLite backup (default `~/.bukio/backups/bukio-<ts>.db`) |
| `restore --from <file> [--to <path>] [--force]` | Restore from a backup file (validated first) |

`restore` refuses to overwrite an existing initialised database unless `--force` is given, and refuses `--from`/`--to` pointing at the same file.

```bash
bukio backup
bukio restore --from ~/.bukio/backups/bukio-2026-08-04T12-00-00.db --to ~/.bukio/restored.db
```

### `bukio audit`

Read the append-only audit log (newest first).

| Option | Default | Description |
|--------|---------|-------------|
| `--since <iso-ts>` | — | Only entries at/after this timestamp (ISO 8601) |
| `--by <who>` | all | Only entries by this actor (e.g. `agent:hermes`) |
| `--limit <n>` | `50` | Max rows |

```bash
bukio audit --by agent:hermes --json   # what did the agent do?
bukio audit --since 2026-08-01         # everything this month
```

---

## Global Flags

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--json` | — | off | Machine-readable JSON output (see below) |
| `--db <path>` | `BUKIO_DB` | `~/.bukio/bukio.db` | Database file |
| `--actor <who>` | `BUKIO_ACTOR` | `human` | Acting entity — use `agent:<name>` when an agent acts |

### JSON output contract

With `--json`, every command prints exactly one JSON document to stdout and exits `0` on success, `1` on failure:

```jsonc
// success
{ "ok": true, "data": { ... } }

// failure
{ "ok": false, "error": { "code": "UNBALANCED", "message": "postings do not sum to zero (sum = 1 cents)" } }
```

All amounts appear both as integer cents (`amount_cents`) and formatted strings (`amount: "1234.56"`). The schema is stable and versioned with the tool — agents can rely on it.

---

## Money Format

- Strict international decimal: `1234.56`, max **2 decimals**, no thousands separators.
- `1234` = 123400 cents; `0.5` = 50 cents.
- **Positive = debit, negative = credit.** A balanced entry's signed amounts sum to zero.
- Thousands separators are rejected on purpose (`1.234` is an error, not 1234) — ambiguity is the enemy of agents.

---

## Integrity & Safety Model

| Guarantee | Enforced by |
|-----------|-------------|
| Postings sum to zero | Engine (creation, in-transaction) + DB trigger (at post time) |
| An entry needs >= 2 postings | Engine + DB trigger (at post time) |
| No zero-amount postings | Engine + `CHECK (amount_cents != 0)` |
| Account codes are 1–6 digits | Engine |
| Account type ↔ normal balance consistency | `CHECK` constraint |
| Postings of a non-draft entry are immutable | DB triggers (INSERT/UPDATE/DELETE) |
| Posted entries are never deleted | Reversal-only workflow + triggers |
| Audit log is append-only | DB triggers (UPDATE/DELETE abort) |
| Money has no floats | Integer cents only, strict parser |
| Single company per database | `CHECK (id = 1)` on `company` |

**Backup:** the database is a single SQLite file (WAL mode). Copy it while the CLI is not writing, or use the `.backup` API / `sqlite3 .backup`:

```bash
sqlite3 ~/.bukio/bukio.db ".backup ~/backups/bukio-$(date +%F).db"
```

A built-in `bukio backup`/`restore` lands in Phase 1.

---

## The Database

- Engine: SQLite (via better-sqlite3), WAL mode, foreign keys on.
- Location: `~/.bukio/bukio.db` by default; override with `--db` or `BUKIO_DB`.
- Migrations: numbered `.sql` files in `migrations/`, applied in order, tracked via `PRAGMA user_version`.

Schema summary (see `migrations/001_initial.sql` for the authoritative DDL):

```
company           — one row (id must be 1): name, kvk, legal_form, btw_id, iban,
                    vat_module, kor_flag, fiscal_year_end
accounts          — chart of accounts: code, name, type, rgs_code, normal_balance, active
journal_entries   — date, description, source, source_ref, state, reversed_from_id,
                    created_by, created_at, posted_at
postings          — entry_id, account_id, amount_cents, document_id
audit_log         — ts, actor, action, command, args_json, outcome, entry_ids
```

---

## Using Agents

bukio-cli is built for agents. The companion file **`AGENTS.md`** in the repo root is the agent's manual: invariants, exact command/JSON contracts, error codes, and worked examples (opening the month, correcting mistakes). Agents should read `AGENTS.md` before driving the tool, and follow the house rules:

1. **Always `--dry-run` before mutating.** Show the plan, then apply.
2. **Always pass `--actor agent:<your-name>`** so the audit trail attributes your work.
3. **Prefer `--json`** for parsing; keep human-readable output for humans.
4. **Never edit the SQLite file directly.** Use the CLI/engine — the triggers and audit log exist for a reason.
5. **Never delete a posted entry.** Reverse it.
6. **Verify after every mutation** (e.g. `report trial-balance --json` must say `balanced: true`).

---

## Project Layout

```
bukio-cli/
├── bin/bukio.js           # CLI entry point
├── src/
│   ├── cli/               # commander commands (init, entry, report, audit, util)
│   ├── core/              # db, accounts, chart, entries (posting engine), money
│   ├── audit/             # append-only audit log
│   └── report/            # trial balance
├── migrations/            # numbered SQL migrations (001_initial.sql)
├── test/                  # node:test suites (unit + CLI end-to-end)
├── AGENTS.md              # agent manual — read before driving the tool
└── README.md
```

---

## Development & Testing

```bash
npm test          # node --test — discovers test/*.test.js
```

The suite covers: money parsing, posting engine invariants, reversal semantics, DB triggers (balance, immutability, append-only audit), trial balance math, and end-to-end CLI flows against temporary databases.

---

## Error Codes

| Code | Meaning |
|------|---------|
| `NO_DATABASE` | No database at the path — run `bukio init` first |
| `ALREADY_INITIALISED` | The database already has a company |
| `INVALID_LEGAL_FORM` | Unknown legal form for `init` |
| `INVALID_FISCAL_YEAR_END` | Fiscal year end must be `mm-dd` |
| `INVALID_RGS_CODE` | RGS code does not match the expected format (e.g. `BMVA.02`) |
| `INVALID_CSV_HEADER` / `EMPTY_CSV` | Chart CSV missing required columns or empty |
| `ALREADY_ACTIVE` / `ALREADY_INACTIVE` | Account already in that state |
| `INVALID_AMOUNT` | Amount string not parseable (see [Money Format](#money-format)) |
| `INVALID_AMOUNT_CENTS` | Posting amount is not a non-zero integer |
| `INVALID_POSTING` | Posting spec is not `CODE:AMOUNT` |
| `INVALID_DATE` | Date is not `yyyy-mm-dd` or not a real calendar date |
| `INVALID_DESCRIPTION` | Description is empty |
| `INVALID_SOURCE` | Unknown source (`manual`/`bank`/`invoice`/`agent` only) |
| `INVALID_ACTOR` | Actor is empty |
| `TOO_FEW_POSTINGS` | Fewer than 2 postings |
| `UNBALANCED` | Postings do not sum to zero |
| `ACCOUNT_NOT_FOUND` | Account code does not exist |
| `ACCOUNT_INACTIVE` | Account exists but is inactive |
| `ACCOUNT_EXISTS` | Account code already exists (account creation, Phase 1) |
| `INVALID_CODE` / `INVALID_NAME` / `INVALID_TYPE` / `INVALID_NORMAL_BALANCE` / `INVALID_COMBINATION` | Account validation (Phase 1 surface) |
| `NOT_FOUND` | Entry id does not exist |
| `ALREADY_POSTED` | Entry is already posted |
| `NOT_POSTED` | Entry must be posted first (reversal) |
| `ALREADY_REVERSED` | A posted reversal already exists for this entry |
| `OUT_REQUIRED` | `--out <path>` is required for xlsx output |
| `FILE_NOT_FOUND` | Backup file does not exist |
| `INVALID_BACKUP` | File is not a valid bukio database |
| `RESTORE_EXISTS` | Target already has a company — pass `--force` |
| `SAME_FILE` | Restore source and target are the same file |
| `INVALID_IBAN` | IBAN is malformed |
| `INVALID_CAMT` / `EMPTY_STATEMENT` | CAMT.053 XML invalid or empty |
| `INVALID_CSV_HEADER` / `EMPTY_CSV` | Bank/chart CSV missing required columns or empty |
| `INVALID_FORMAT` | Unknown `--format` for bank import |
| `NOT_FOUND` (bank) | Bank transaction does not exist |
| `ALREADY_MATCHED` | Bank transaction already matched/ignored |
| `VAT_MODULE_OFF` | VAT module not enabled for this company (`vat enable` first) |
| `KOR_ACTIVE` | KOR company cannot enable the VAT module |
| `VAT_CODE_NOT_FOUND` | `@CODE` references an unknown VAT code |
| `VAT_MARGIN_NOT_SUPPORTED` | Margeregeling cannot be split automatically |
| `INVALID_PERIOD` | Period must be `YYYY-Qn` or `YYYY-MM` |
| `SQLITE_CONSTRAINT_TRIGGER` | A database trigger aborted the operation (e.g. editing a posted entry, rewriting the audit log) |

---

## Common Tasks

**Open a company's books**
```bash
bukio init --name "Demo BV" --kvk 12345678 --legal-form bv --vat on
bukio entry add --desc "Startkapitaal" --postings "1100:10000.00,3000:-10000.00" --post
```

**Book an expense** (paid from the bank account)
```bash
bukio entry add --desc "Kantoorartikelen" --postings "4300:250.00,1100:-250.00" --post
```

**Book sales** (money received, income)
```bash
bukio entry add --desc "Factuur 2026-001" --postings "1100:1210.00,8000:-1210.00" --post
```

**Correct a mistake** — reverse, then book correctly:
```bash
bukio entry reverse --id 2 --reason "verkeerde categorie"
bukio entry add --desc "Kantoorartikelen (gecorrigeerd)" --postings "4200:250.00,1100:-250.00" --post
```

**Month-end sanity check**
```bash
bukio report trial-balance --year 2026 --json   # must be balanced: true
bukio report balans --as-of 2026-12-31          # must say BALANCED
bukio report pnl --year 2026                    # result = revenue - costs
bukio audit --since 2026-08-01 --by agent:hermes
```

**Hand the year to your boekhouder**
```bash
bukio report journal --year 2026 --format xlsx --out ~/exports/journal-2026.xlsx
bukio report balans --as-of 2026-12-31 --format csv --out ~/exports/balans-2026.csv
bukio report pnl --year 2026 --format xlsx --out ~/exports/pnl-2026.xlsx
```

**Month-end close with bank + VAT (the real workflow)**
```bash
# 1. import the bank statement (idempotent — safe to re-run)
bukio bank import --file ~/exports/rabo-2026-06.camt.xml --iban NL91ABNA0417164300
# 2. dry-run the auto-match, then apply
bukio bank match auto --dry-run
bukio bank match auto
# 3. handle the leftovers: suggest -> post or link
bukio bank match suggest
bukio bank match post --tx 17 --account 4300
# 4. the balance check: bank balance must equal the ledger balance
bukio bank list
bukio report trial-balance --json          # must be balanced: true
# 5. VAT quarter: read the OB fields, file manually in Mijn Belastingdienst
bukio vat readout --period 2026-Q2
bukio vat readout --period 2026-Q2 --mark-filed
```

**Protect the books**
```bash
bukio backup                              # ~/.bukio/backups/bukio-<ts>.db
bukio restore --from ~/.bukio/backups/bukio-....db --to ~/.bukio/test-restore.db
```

**Extend the chart of accounts**
```bash
bukio account add --code 4350 --name "Reiskosten" --type expense --normal-balance debit --rgs-code WBED.42
bukio account import --file assets/chart-nl.csv --dry-run
```

**Run two companies** — separate databases:
```bash
bukio --db ~/.bukio/bv-a.db init --name "BV A" --legal-form bv
bukio --db ~/.bukio/bv-b.db init --name "BV B" --legal-form bv
```

**KOR / non-VAT entity** — simply omit the VAT module; the ledger never exposes VAT concepts:
```bash
bukio init --name "Mijn ZZP" --kor
```

---

## Roadmap

| Phase | Scope | Status |
|-------|-------|--------|
| 0 | Foundation: ledger, posting engine, audit, trial balance, `--json`/`--dry-run` | ✅ done |
| 1 | Accounts CRUD + CSV import, RGS-mapped chart, balans + W&V, CSV/XLSX export, backup/restore | ✅ done |
| 2 | Bank import (CAMT.053/CSV), matching; optional VAT module (codes, OB readout, KOR) | ✅ done |
| 3 | Invoicing: factuurvereisten, PDF (Playwright), UBL/Peppol BIS 3.0, credit notes | next |
| 4 | Jaarrekening micro/klein models, closing entries, KVK package, ICP readout | planned |
| 5 | Agent layer: MCP server, permissions, NL query, AI categorization suggestions, compliance calendar | planned |
| 6 | Optional: Ponto live feeds, Peppol send/receive, OCR, SQLCipher | optional |

Design principles persist across phases: **agent-native from day one**, **VAT optional**, **no automated tax filing**, **single company per database**, **local-first**.

---

literal:*Part of the Bukio product line — separate from the Bukio web platform: shared brand and philosophy, no shared code.*
