<div align="center">

<a href="https://agentic.bukio.nl"><img src="https://agentic.bukio.nl/assets/logo-light.svg?v=2" alt="Bukio" width="340"></a>

**Agent-first double-entry bookkeeping for SMEs across sixteen jurisdictions.**

VAT-optional · Peppol BIS 3.0-ready · Local-first (SQLite) · MCP-native

[![Website](https://img.shields.io/badge/website-agentic.bukio.nl-2b6cb0)](https://agentic.bukio.nl)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Version](https://img.shields.io/github/package-json/v/erikvankempen/bukio-cli?label=version&color=2b6cb0)](https://github.com/erikvankempen/bukio-cli/releases)
[![Node](https://img.shields.io/badge/node-%3E%3D20-brightgreen)](package.json)
[![Tests](https://img.shields.io/badge/tests-932%20passing-brightgreen)](test/report.md)
[![Peppol](https://img.shields.io/badge/Peppol-BIS%203.0%20ready-orange)](https://peppol.eu/)
[![MCP](https://img.shields.io/badge/MCP-server-blueviolet)](#using-agents)

</div>

bukio-cli is a double-entry bookkeeping engine and CLI that runs natively on a VPS, stores everything in one local SQLite file, and is designed so AI agents — not just humans — can operate it safely and auditably. It is built for the EU B2B e-invoicing wave: every invoice ends as a compliant PDF, a Peppol BIS 3.0 UBL document, and a sendable Peppol message — in any of the sixteen supported jurisdictions (NL, LU, GB, FR, US, BE, DE, DK, FI, NO, SE, AT, IE, IT, ES, PT).

**Proven in production:** bukio-cli currently runs a live Dutch company's books, operated end-to-end by [Hermes Agent](https://hermes-agent.nousresearch.com) (Nous Research) running **DeepSeek V4 Flash** via OpenCode Go on a Linux VPS — the same stack every day: bank imports, invoice booking, month-end close checks and statutory reports, every action attributable in the audit log. The full stack disclosure is in [AI Development Cost & Token Usage](#ai-development-cost--token-usage).

## Features

- **Agent-native** — every command emits deterministic `--json`; every mutation supports `--dry-run` (plan mode); every action lands in an append-only audit log with named-actor attribution (`--actor agent:bartholomeus` / `human:erik`).
- **VAT optional** — the core ledger is VAT-agnostic. The optional VAT module adds codes, the VAT-return readout (the NL profile's statutory OB shape — fields 1a–5d) and KOR support when you need them. Filing always stays manual — bukio never submits anything.
- **Peppol BIS 3.0 ready** — the 2027 mandate both ways: `finalize → PDF → UBL → peppol-send` for outgoing, and `import invoice` (EN 16931/Peppol UBL) into the payables register for incoming. `invoice email` delivers the PDF by SMTP (`BUKIO_SMTP_*` env).
- **Documents in the DB** — `attach` stores source documents (PDFs, scans) as BLOBs by default (metadata-only lists; 25 MB/file cap; sha256 dedupe) or content-addressed files, so the books carry their paper trail and backups stay one consistent file. `backup --encrypt` (AES-256-GCM) + `--keep N` rotation protects it off-box.
- **Cash management reports** — `report aging` (debtors/creditors 30/60/90+ buckets), `contact statement` (opgave with running balance), `report sales --by contact|item` for the agent's weekly briefings.
- **FX built in** — book foreign-currency purchase invoices in USD, GBP, …; rates resolve from your rate store or straight from the ECB.
- **Migration-ready** — `import opening-balances`, `import journal` (SnelStart/Exact-style CSV) and `import xaf` (XML Auditfile Financieel 4.0) bring a whole administration in; every importer validates the entire file before writing a single cent.
- **Runs itself** — `month-end` is the agent's close check (drafts, bank, VAT, invoices, recurring, fixed assets, profit); `invoice reminders` drafts overdue payment reminders.
- **Fixed assets** — depreciation schemes (lineair / degressief with the standard switch-to-linear rule), an asset register with **mid-life adoption** (recognition date + cumulative depreciation at recognition — only the remaining depreciation is booked), monthly runs (idempotent per asset-month), disposal with winst/verlies booking, and the **activastaat** (CSV/XLSX export).
- **SEPA payment batches** — a payables register (purchase invoices, `transfer` vs `direct_debit`/incasso), batch creation from unpaid invoices or CSV, and **pain.001 export** (`001.03`/`001.09`) for upload in any SEPA bank portal. **Direct debit** adds a direct-debit mandate register (`payments mandate add`, core/b2b) and **pain.008.001.02 export** (one `PmtInf` per scheme, FRST/RCUR auto-assigned). One export per batch (unique `MsgId` — re-uploading would double-pay); the ledger is untouched until the bank statement import books the payments.
- **One company per database** — a second company is a second SQLite file (`--db` or `BUKIO_DB`).
- **Local-first** — no cloud, no lock-in. Your 7-year administration stays yours.
- **Sixteen jurisdictions** — `bukio init --country <cc>` seeds the country's chart convention (RGS, PCN 2020, PCG, SKR 03, BAS 2023, NS 4102, EKR, PGC, SNC, …), VAT codes/rates, identifiers and compliance calendar (NL, LU, GB, FR, US, BE, DE, DK, FI, NO, SE, AT, IE, IT, ES, PT — see [Supported jurisdictions](#supported-jurisdictions)). Format dispatch is strict: unbuilt markets fail loudly (`FORMAT_NOT_SUPPORTED`), never silently fall back.
- **Localization (i18n)** — optional and opt-in: `--locale <code>` / `BUKIO_LOCALE` switches human-facing output to Dutch, Belgian Dutch, German, French, Luxembourg French, Danish, Finnish, Norwegian or Swedish; **English is the default** whenever localization is off (see [Localization](#localization-i18n)). JSON, error codes and statutory documents never localize.

## Quick start

Let your agent do it. Paste this prompt to any agentic assistant — the agent
installs from source and stops before touching any financial data:

```
Install bukio-cli from github.com/erikvankempen/bukio-cli.

Verify Node.js 20+ and a Linux or macOS environment, then clone the repository, run npm install and npm link, and confirm with bukio --version.

Read the repository README.md and AGENTS.md files, configure `bukio mcp` as a local stdio MCP server, and explain the setup you made. Do not create a company or book real transactions yet. When we start, use named actors, preview every mutation with --dry-run, and ask for confirmation before writing.
```

## Screenshot

<p align="center">
  <img src="screenshot.png" alt="bukio-cli demo session" width="720">
</p>

## Table of Contents

1. [Features](#features)
2. [Quick start](#quick-start)
3. [Screenshot](#screenshot)
4. [Requirements & Install](#requirements--install)
5. [Core Concepts](#core-concepts)
6. [Command Reference](#command-reference)
7. [Global Flags](#global-flags)
8. [Localization (i18n)](#localization-i18n)
9. [Money Format](#money-format)
10. [Integrity & Safety Model](#integrity--safety-model)
11. [The Database](#the-database)
12. [Using Agents](#using-agents)
13. [Scheduling recurring actions (cron)](#scheduling-recurring-actions-cron)
14. [Project Layout](#project-layout)
15. [Development & Testing](#development--testing)
16. [Error Codes](#error-codes)
17. [Common Tasks](#common-tasks)
18. [EU AI Act Transparency](#eu-ai-act-transparency)
19. [AI Development Cost & Token Usage](#ai-development-cost--token-usage)
20. [Troubleshooting](#troubleshooting)
21. [Supported jurisdictions](#supported-jurisdictions)
22. [Roadmap](#roadmap)

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

Accounts are organised in a **chart of accounts** with 4-digit codes and a type — the example below is the NL profile's default chart (the initial market); every other profile seeds its own convention (PCN 2020, PCG, SKR 03, BAS 2023, NS 4102, ...):

| Type | Normal balance | Examples |
|------|----------------|----------|
| `asset` | debit | 1000 Kas, 1100 Bank, 1200 Debiteuren |
| `liability` | credit | 2000 Crediteuren, 2100 Overige schulden |
| `equity` | credit | 3000 Eigen vermogen |
| `expense` | debit | 4000 Inkoopwaarde, 4100–4500 kosten |
| `income` | credit | 8000 Omzet, 8100 Overige opbrengsten |

`bukio init` seeds the country profile's default chart — the NL profile's 29-account RGS-mapped chart (Referentie Grootboekschema; account codes RGS-compatible in structure), other profiles their own conventions — plus the profile's VAT codes/rates, identifiers and compliance calendar.

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

Every mutation records an **actor** — every command requires a named identity in the form `'<role>:<name>'`: `human:erik` when you act yourself, `agent:bartholomeus` when an agent acts. A bare `human` or `agent` is rejected. Actors appear on entries (`created_by`) and in the audit log, so a human can always see exactly what an agent did.

### Actor identity & signing

Every actor can hold an **Ed25519 key pair**. Once an actor has a key and it is enrolled in a company database, every command that actor runs is digitally **signed** — the signature covers a canonical digest of the command, its arguments, a timestamp and a one-time nonce, and is stored on the audit-log row for that action. `bukio audit verify` recomputes the digest and re-checks every signature against the company's key registry, so a tampered audit log or a forged command is detectable after the fact.

- **Keys** — `bukio actor keygen` writes `<config>/keys/<role>-<name>.key`. Agent and system keys are plain files; **human keys are passphrase-encrypted** and are unlocked per session with `bukio actor unlock` (12 h by default, `--ttl-hours` to change) or by setting `BUKIO_SIGNING_PASSPHRASE`. `bukio actor lock` clears the session.
- **Enrolment is per company** — `bukio actor register` enrols the actor's local key into the current company's database (`actor_keys`). An actor working in several companies repeats this for each database; every company's registry and enforcement state are independent. **Enrol before you enforce**: under enforcement, registering a brand-new actor is refused (operator-gated — an *enrolled* actor flips `enforce --off` → `register` → `--on`, each audited); re-enrolment after a *revocation* (rotation) remains possible under enforcement.
- **Enforcement** — signing is *recorded* by default (`record` mode: signed commands log `verified`, unsigned ones still run and log `unsigned`). `bukio actor enforce --on` makes a valid signature **required**: unsigned or unverifiable commands are refused before anything is written — including through the MCP server, whose mutating tool calls are signed the same way. **Only an enrolled actor can disable enforcement** (`actor enforce --off`); the trade-off is deliberate: if every enrolled key is lost, the CLI is locked out on that company (recover via backup, or the owner edits the DB directly).
- **Revocation & rotation** — `bukio actor revoke --reason …` revokes the actor's key in the current company (the row is retained as history, so audit rows signed with it stay verifiable as `revoked`). Rotate with `bukio actor keygen --force` + `actor register` — the fresh key is enrolled as a new registry row and historical rows remain provable.
- **Explicit key** — `--sign-key <path>` (or the session key / `BUKIO_SIGNING_PASSPHRASE` / the actor's key file, in that order) chooses which private key signs the command.

**In practice — what you actually do:**

```bash
# ONE-TIME, per identity you act as:
bukio --actor human:erik actor keygen        # human: prompts for a passphrase
bukio --actor human:erik actor register      # enrol into THIS company's DB
BUKIO_ACTOR=agent:bartholomeus bukio actor keygen
BUKIO_ACTOR=agent:bartholomeus bukio actor register   # repeat per company DB

# PER SESSION, humans only (their keys are encrypted at rest):
bukio --actor human:erik actor unlock        # 12 h by default; actor lock clears it

# THAT'S IT — every command now signs itself automatically.
# No signature to type, nothing to pass: the gate signs before executing
# (reads, writes and --dry-run alike), stamps the audit row verified, and
# `bukio audit verify` can re-prove the whole trail at any time.
```

Agent/system keys (plain files) and sessions sign with **zero ceremony** — an agent or cron job that has the key file on disk is signed automatically. Enforcement (`actor enforce --on`) only adds a requirement: unsigned or unverifiable commands are refused *before anything is written*, instead of running logged as `unsigned`. See AGENTS.md §6.20 for the full walkthrough and the refusal→fix table.

> **Strong recommendation: one OS user per agent.** Signing proves *which key* signed — not *which process* used it. Two agents running under the same OS user can read each other's key files (`~/.bukio/keys/…`) and impersonate each other undetectably. If you run several agents on one machine, give each its own OS account (or container) with its own `BUKIO_CONFIG_DIR`, so the filesystem itself keeps the keys apart. This is the same trust boundary SSH keys have — the OS user is the real security boundary.

> **Confinement: an agent that can write the code can remove the gates.**
> Signing and authorizations protect the *books*, not the *CLI itself*:
> a process with write access to the source can strip the signature
> checks, authz gates and enforcement — and no check *inside* the code
> can stop it, because the verifier lives in the same file the attacker
> can edit. Confinement is the only real prevention:
>
> - **Run each agent as its own non-root OS user.** The same boundary
>   that keeps keys apart keeps the code apart: an agent user without
>   write permission on the install cannot alter what it executes.
> - **Install the CLI read-only.** Keep a release copy at a root-owned,
>   agent-unwritable path (e.g. `/opt/bukio-cli`, `0755 root:root`, or a
>   read-only bind mount / container volume) and put *that* on the
>   agent's PATH. Development happens in the writable working tree; the
>   agent runs the immutable install.
> - **Detection as the backstop.** Verify installed files against a hash
>   manifest stored on a *second* host (nightly cron), so any drift
>   alerts you. That detects tampering after the fact — it does not
>   prevent it. Prevention is the read-only install.

### Authorizations & segregation of duties

Signing proves **who** ran a command; authorizations control **what kinds
of commands** an actor may run. `bukio actor authz --on` turns on
**per-actor authorizations** for a company: from then on every command
needs a role granting its capability — deny-by-default, refused with
`AUTHZ_DENIED` in the sign gate *before anything is written* (dry-run
included). This is how segregation of duties (SoD) is enforced: the actor
who drafts is not the one who posts, the one who books is not the one who
files tax or authorises payments.

The granularity is **capability families + roles**, not a per-command
matrix. Roles: `owner` (everything), `bookkeeper` (chart, entries,
contacts, invoices, bank import+match, VAT booking, assets, recurring, FX,
reports), `payments` (bank import+match, SEPA money-out, reports), `tax`
(VAT booking+filing, month/year close, XAF export, reports), `assets`,
`readonly`. The role→capability map lives in the code, versioned with the
binary — one grant line the owner can read:

```bash
# one-time bootstrap: authz --on implies enforcement and makes the
# flipper the owner (enrol all agents BEFORE flipping — first enrolment
# is refused under enforcement)
bukio actor authz --on
bukio actor roles grant bookkeeper --for agent:invoicing
bukio actor roles grant payments    --for agent:payments
# conflicts warn softly (bookkeeper+payments, bookkeeper+tax, ...)
# the review lens: who can run this command?
bukio actor who-can 'entry post'
```

- Grants are **owner-only** under authz; listing your own roles
  (`actor roles`) and checking your own commands (`actor can '<cmd>'`)
  are self-service, so a refused agent can always see why.
- A **conflicting grant warns but does not block** — legitimate
  single-operator setups exist (the owner has everything anyway); the
  warning keeps the review in front of the eye, `who-can` makes the
  matrix visible.
- **Owner-mediated key revoke**: `actor revoke --target <who> --reason`
  lets the operator kill a compromised agent key — owner role required
  **regardless** of authz mode.
- The MCP server gates its mutating tool calls with the **same**
  capabilities (read-only tools are not gated). `entry_add` with
  `post:true` needs `entry.post`, like the CLI.
- Turn it off with `actor authz --off` (owner only); signing enforcement
  stays on, roles become inert configuration. The last owner can never be
  revoked, so authz can always be turned off.

Full walkthrough: AGENTS.md §6.21.

### The audit log

An **append-only** log of every mutation: actor, action, command, JSON args, outcome, and affected entry IDs. Database triggers block UPDATE and DELETE — the log cannot be rewritten after the fact. Read it with `bukio audit`.

### Amounts

All money is stored as **integer cents** (`amount_cents`). There are no floats anywhere in financial code paths. See [Money Format](#money-format).

---

## Command Reference

Global flags (`--json`, `--db`, `--actor`) can appear before or after the subcommand. See [Global Flags](#global-flags).

### `bukio init`

Initialise a company database: creates the file, the company row, and seeds the default chart of accounts.

| Option | Default | Description |
|--------|---------|-------------|
| `--name <name>` | *(required)* | Company name |
| `--country <cc>` | `NL` | Country profile: `NL`, `LU`, `GB`, `FR`, `US`, `BE`, `DE`, `DK`, `FI`, `NO`, `SE` |
| `--registration-id <id>` | — | Company registration number (identifier type follows the profile: KVK, KBO, CRN, SIREN, RCS, CVR, Y-tunnus, Org.nr.; deprecated alias `--kvk`) |
| `--legal-form <form>` | profile default | Legal forms follow the profile (NL: `eenmanszaak` \| `vof` \| `bv` \| `nv` \| `stichting` \| `vereniging`; DE: `gmbh` \| `ug` \| `ag` \| ...; US: `llc` \| `c-corp` \| `s-corp` \| ...) |
| `--tax-id <id>` | — | Tax/VAT identification number (deprecated alias `--btw-id`) |
| `--iban <iban>` | — | Bank account (IBAN) |
| `--vat <on\|off>` | `off` | Enable the VAT module (Phase 2) |
| `--kor` | off | Small business scheme — implies `--vat off` |
| `--fiscal-year-end <mm-dd>` | `12-31` | Fiscal year end |
| `--dry-run` | off | Show the plan without writing anything |

Fails with `ALREADY_INITIALISED` if the database already has a company.

```bash
bukio init --name "Demo BV" --country NL --registration-id 12345678 --legal-form bv --vat on --dry-run
bukio init --name "Demo BV" --country NL --registration-id 12345678 --legal-form bv --vat on
bukio init --name "Muster GmbH" --country DE --registration-id HRB123456 --legal-form gmbh --vat on
```

### `bukio company`

Company record — the supplier gegevens on your invoices (12-vereisten 1–3 must be complete before `invoice finalize`).

| Command | Purpose |
|---------|---------|
| `company show` | Current company record (name, registration id, tax id, iban, address) |
| `company update --name --registration-id --tax-id --iban --address --postal-code --city [--dry-run]` | Update supplier data (audited; IBAN mod-97 validated; deprecated `--kvk`/`--btw-id` aliases) |
| `company update --logo FILE` / `--remove-logo` / `company logo --out FILE` | Store/extract the invoice logo (PNG/JPEG/SVG ≤ 1 MB, ≤ 2048×2048 px, stored as a BLOB in the DB — travels with backups) |

```bash
bukio company update --address "Industrieweg 12" --postal-code "2712 CD" --city "Zoetermeer" --btw-id NL123456789B01
bukio company update --logo ~/logo.svg
bukio company show
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
| `--state <state>` | all | `draft` \| `posted` |
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

### `bukio report balance-sheet`

Balance sheet as of a date, grouped by the profile's taxonomy (NL example — RGS hoofdgroep: Materiële vaste activa, Voorraden, Vorderingen, Liquide middelen / Eigen vermogen, Kortlopende schulden, …). Includes the computed **undistributed result** (net result of income/expense accounts). Invariant: **total assets = total liabilities + equity + result** — the report says `BALANCED` or `UNBALANCED!`. (`balans` is a deprecated alias.)

| Option | Default | Description |
|--------|---------|-------------|
| `--as-of <yyyy-mm-dd>` | today | Balance date (inclusive) |
| `--format <format>` | human (json with `--json`) | `json` \| `csv` \| `xlsx` \| `human` |
| `--out <path>` | stdout | Output file (required for xlsx) |

### `bukio report pnl`

Profit and loss statement for a period, grouped by the profile's taxonomy (NL example — RGS hoofdgroep: Omzet, Inkoopwaarde van de omzet, Personeelskosten, Afschrijvingen, Overige bedrijfskosten, Financiële baten en lasten, …). Reports revenue, costs and **net result**.

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
bukio report balance-sheet --as-of 2026-12-31
bukio report pnl --year 2026 --format xlsx --out ~/exports/pnl-2026.xlsx
bukio report journal --year 2026 --format csv --out ~/exports/journal-2026.csv
```

### `bukio report aging` / `report sales` / `contact statement`

Open-items and revenue analytics (v0.14) — all exportable with `--format csv|xlsx [--out]`.

| Command | Purpose |
|---------|---------|
| `report aging [--as-of D] [--kind debtors\|creditors\|both]` | Open items per contact bucketed by days past due (current/30/60/90+); creditors show `in_batch` amounts separately — the agent's daily cash pulse |
| `report sales --year YYYY [--by contact\|item]` | Sales revenue: per contact (net/vat/gross via the totals engine) or per item (net after per-line discounts; invoice-level discounts are not allocated per line) |
| `contact statement --id N [--as-of D]` | Statement (alias: `opgave`): the contact's invoices + payments + payables with a running balance (positive = they owe you) |

```bash
bukio report aging --kind debtors --format csv --out ~/exports/aging.csv
bukio report sales --year 2026 --by contact
bukio contact statement --id 3
```

### `bukio account`

Chart of accounts management.

| Command | Purpose |
|---------|---------|
| `account add --code <c> --name <n> --type <t> --normal-balance <d\|c> [--taxonomy-code <r>] [--dry-run]` | Add an account (deprecated alias `--rgs-code`) |
| `account list [--type <t>] [--include-inactive]` | List accounts |
| `account show --code <c>` | Show one account |
| `account deactivate --code <c>` | Deactivate (blocks new postings; history stays) |
| `account reactivate --code <c>` | Reactivate |
| `account import --file <chart.csv> [--dry-run]` | Import a chart from CSV: `code,name,type,normal_balance[,taxonomy_code]` (legacy `rgs_code` header still accepted) |

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
| `bank import --file <path> --iban <IBAN> [--format camt\|csv\|auto] [--name] [--account-code] [--dry-run]` | Import transactions — **CAMT.053 XML** or bank CSV (NL bank formats — Rabo/ING/ABN column aliases, Dutch `1.234,56` amounts, Af/Bij sign; other markets' formats follow the profile). Idempotent via SHA-256 hash. |
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
| `vat codes` | List VAT codes (the NL profile's: 21, 9, 0, V vrijgesteld, R/RE verlegd, M marge, P privé; other profiles seed their own) |
| `vat book --date --desc --postings "1100:121.00,8000:-100.00@21" [--post] [--dry-run]` | Book a VAT-aware entry. `@CODE` tags a posting as net; the VAT amount and the VAT ledger leg (2500/1500) are computed automatically. |
| `vat readout --period 2026-Q2 [--mark-filed]` | **VAT-return manual-filing readout** (the NL statutory OB shape — fields 1a–5d) for the period (quarter `YYYY-Qn` or month `YYYY-MM`). bukio never files; you enter these amounts in Mijn Belastingdienst Zakelijk. `--mark-filed` records the filing. |
| `vat file --period 2026-Q2 [--account 2510] [--dry-run]` | Reclassify the outstanding VAT position to **'Af te dragen omzetbelasting'** (default 2510, auto-created) at filing — clears 1500/2500, moves the exact-cents net. If the requested code is taken by another account (e.g. an imported chart), it auto-falls to the next free numeric code (2511, …) and reports it; pick any free code with `--account`. The form is filed in rounded whole euros; the cents-level difference is settled to the P&L later. |
| `vat settle --tx <id> [--period] [--account 2510] [--difference-account 4700] [--dry-run]` | Book the bank payment that cancels the af-te-dragen balance (tx must be unmatched; outgoing for te betalen, incoming for a refund). `--account` matches what `vat file` used (default 2510; pass 2511 when the filing collided to the next free code). The rounding difference between the filed whole-euro payment and the exact-cents liability goes to the P&L difference account (default 4700, `--difference-account` for a dedicated account) — a gain when you rounded in your favour. Entry + tx reconciliation commit atomically. |

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

**OB field mapping (the NL statutory return shape):** 1a/1b/1c omzet (21%/9%/0%/vrijgesteld), 1d privégebruik, 3a/3b/3c inkopen, 4a/4b verlegde btw (binnenland/EU, netted via 5b), 5a verschuldigde btw, 5b voorbelasting, 5d te betalen/te ontvangen. Fields 2a/2b (exports) and 5c are not tracked in Phase 2 (shown as 0).

### `bukio recurring` / `bukio depreciation`

Recurring entries & period automation (FR3A) — deterministic, dry-run first, fully audited. Templates are validated at creation; generation just replays them. bukio never generates entries on its own: the agent or a cron job triggers `run --due`.

| Command | Purpose |
|---------|---------|
| `recurring add --name N --postings "CODE:AMT,..." --frequency monthly\|quarterly\|yearly --start YYYY-MM-DD [--day 1-28] [--end] [--runs] [--reverse-previous] [--dry-run]` | Create a recurring **entry** template (VAT-aware via `@CODE` tags; expanded at creation) |
| `recurring add --name N --kind invoice --contact N --lines "2x DESC @ PRICE @21" --frequency monthly --start YYYY-MM-DD [--due-days 30]` | Create a **subscription invoice** template — each run generates a DRAFT invoice (never auto-finalizes; the agent finalizes) |
| `recurring list [--status active\|paused\|completed\|all]` / `show --id` | Inspect templates |
| `recurring pause --id` / `resume --id` | Control scheduling |
| `recurring preview [--as-of DATE] [--template ID]` | What is due (read-only plan) |
| `recurring run [--as-of DATE] [--template ID] [--dry-run]` | Generate all due entries/invoice drafts — **backfills missed periods**, idempotent, one transaction per template (a failing template is reported and skipped, others still run) |
| `depreciation add --name N --cost C --life-months M --start DATE [--asset 1800] [--expense 4600] [--residual 0] [--dry-run]` | Linear monthly depreciation with a **remainder-adjusted final run** (cents-exact total over the asset life) |

**Semantics:**
- Generated entries: `source='recurring'`, `source_ref='tpl:<id>'`, `created_by='recurring'` (the trigger actor is in the audit log). Posted, immutable, reversible like any entry.
- `--reverse-previous` implements the accrual pattern: each run first reverses the previous generated entry (contra-entry dated at the original), then books the new one — monthly estimates replace cleanly, each month carries its own amount.
- `--runs` / `--end` complete the template (status `completed`); a completed template cannot be re-activated.
- First run is normalized to `--day` (never backwards); days 29–31 are rejected to avoid month-end clamping.

```bash
# depreciation: 5370.00 over 36 months -> 149.17/mo, final 149.05 (total exactly 5370.00)
bukio depreciation add --name "Laptop Dell" --cost 5370.00 --life-months 36 --start 2026-08-01
# accrual with auto-reversal (nog te betalen kosten, monthly estimates)
bukio recurring add --name "Nog te betalen kosten admin" \
  --postings "4310:250.00,2400:-250.00" --frequency monthly --start 2026-08-31 --day 28 --reverse-previous
# prepaid spreading: annual insurance over 12 months
bukio recurring add --name "Verzekering 12 mnd" \
  --postings "4320:100.00,1700:-100.00" --frequency monthly --start 2026-08-01 --runs 12
# the agent's month-end: preview, then run
bukio recurring preview --as-of 2026-09-30
bukio recurring run --as-of 2026-09-30
# subscription invoices: run generates DRAFT invoices, then the agent finalizes
bukio recurring add --name "SaaS abonnement" --kind invoice --contact 1 \
  --lines "2x Premium SaaS @ 99.00 @21" --frequency monthly --start 2026-08-01 --due-days 14
bukio recurring run --as-of 2026-10-31        # -> draft invoices 2026-08/09/10
bukio invoice finalize --id 1                 # -> 2026-0001, booked
bukio invoice peppol-send --id 1 --dry-run    # Peppol access-point (env creds)
```

### `bukio contact` / `bukio invoice`

Outgoing invoicing (FR3) — compliant with the 12 verplichte factuurvereisten, lifecycle draft → sent → paid (overdue derived), credit notes, PDF + UBL export, bank payment matching.

| Command | Purpose |
|---------|---------|
| `contact add --name N [--address] [--postal-code] [--city] [--vat-id] [--registration-id] [--email]` | Add a customer (VAT id required when reverse charge) |
| `contact list` | List contacts |
| `item add --name N [--description] [--unit h\|day\|month\|unit\|session\|km\|kg\|project] --price P [--vat] [--gl]` / `item list` / `item show --id` / `item update --id [--price] [--unit] [--vat] [--gl] [--deactivate]` | **Items catalog (v0.13)**: reusable products/services; invoice lines snapshot the price/VAT at creation, so later edits never rewrite existing invoices; `--deactivate` blocks new invoices (existing keep their snapshots) |
| `invoice create --contact <id> --lines "2x Consultancy @ 150.00 @21,1x Rapportage @ 400.00 @9" --date YYYY-MM-DD [--due-days 30] [--delivery-date] [--description] [--reference] [--dry-run]` | Create a draft invoice. Line spec: `[QTYx] DESC @ PRICE [@ VATCODE] [@ -DISCOUNT]` — fractional quantities (`1.5x`), per-line discounts (`@-10%` or `@-25.00`) |
| `invoice create --contact <id> --items "1:2,3:1.5@140.00@21@-10%" ...` | Create from the catalog — item spec `ID[:QTY][@PRICE][@VATCODE][@-DISCOUNT]`; price/VAT overrides apply to **this invoice only** (catalog untouched) |
| `invoice create ... [--discount-pct 5 \| --discount-amount 50.00] [--language nl\|en]` | **Total discount** (before VAT; allocated across VAT-rate groups to the cent so the VAT-return readout reconciles) and invoice language — follows the company profile (Dutch for NL/BE companies, English otherwise), `nl|en` explicit override — PDF labels and unit names |
| `invoice finalize --id N [--dry-run]` | **Assign the sequential number (YYYY-NNNN) and book the entry** (Debiteuren / Omzet / Te betalen btw) |
| `invoice list [--status] [--type]` / `show --id` | Inspect invoices |
| `invoice pdf --id N [--out PATH]` | Render a compliant PDF via headless Chromium — includes the **company logo** (set via `company update --logo`), a **VAT breakdown per rate** (`Btw 21% over …`), a Eenheid/Unit column and localized labels |
| `invoice ubl --id N [--out PATH]` | Export **UBL 2.1 / Peppol BIS 3.0 (EN 16931)** XML |
| `invoice credit --id N [--reason]` | Create a credit note (draft) from a finalized invoice (inherits language + discounts) |
| `invoice pay --id N --date [--amount]` | Record a payment (tracking; the posting comes from the bank flow) |
| `contact statement --id N [--as-of D]` | **Statement (alias `opgave`, v0.14)**: the contact's invoices + payments + payables with a running balance |
| `invoice email --id N [--to] [--subject] [--body] [--no-pdf] [--dry-run]` | **Email the finalized invoice PDF (v0.14)** via SMTP (`BUKIO_SMTP_HOST/PORT/USER/PASS/FROM` env). Delivery is audited (`invoice.email`); dry-run renders + validates but sends nothing. Status is `sent` from finalize onward |
| `invoice peppol-send --id N [--endpoint] [--dry-run]` | POST the UBL to a Peppol access-point provider (`BUKIO_PEPPOL_ENDPOINT` + `BUKIO_PEPPOL_TOKEN` env) |

**Compliance (validated at finalize):** supplier name/registration id/tax id/address/postal/city (set at `init`), invoice date, sequential number, customer name+address+city, line descriptions/quantities/prices, VAT rate + amount per rate, totals, and the customer's VAT id when a line carries `@R`/`@RE` (reverse charge). Missing data fails with `SUPPLIER_INCOMPLETE` / `CUSTOMER_INCOMPLETE` / `CUSTOMER_VAT_REQUIRED`.

**Payment matching:** `bank match auto` now recognizes incoming payments against unpaid sales invoices (exact outstanding amount, oldest due first) — it marks the invoice paid, posts Bank/Debiteuren, and reconciles the transaction. The VAT-return readout picks up invoiced sales automatically.

```bash
bukio invoice create --contact 1 --date 2026-07-10 \
  --lines "2x Consultancy @ 150.00 @21,1x Rapportage @ 400.00 @9" --reference "PO-2026-88"
bukio invoice finalize --id 1 --dry-run      # plan: number + postings
bukio invoice finalize --id 1                # -> 2026-0001, entry posted
bukio invoice pdf --id 1                     # 2026-0001.pdf
bukio invoice ubl --id 1                     # 2026-0001.xml (Peppol BIS 3.0)
# payment arrives -> the bank import matches it automatically
bukio bank import --file stmt.xml --iban NL91ABNA0417164300
bukio bank match auto                        # tx -> invoice 2026-0001 (paid)
```

### `bukio year-end` / `bukio jaarrekening` / `bukio icp`

Annual close and statutory reporting (Phase 4).

| Command | Purpose |
|---------|---------|
| `year-end status --year YYYY` | Open/closed, the year's result, per-account nets |
| `year-end close --year YYYY [--dry-run]` | **Close the fiscal year**: reverse income/expense into 9900 (created on demand), then resultaatbestemming into 3000. Both entries `source='closing'`, `source_ref='fy:YYYY'`. Guards: draft entries in the year (`INCOMPLETE_YEAR`), double close (`ALREADY_CLOSED`), no activity (`EMPTY_YEAR`). **The P&L report excludes closing entries** — the year's flow stays visible after closing; the balance sheet then shows equity including the result |
| `financial-statements report --year YYYY --model <model> [--format json\|pdf\|xlsx] [--out]` | Statutory annual accounts — models per the country profile (NL: `micro`\|`klein`, Dutch layout, Titel 9 Boek 2 BW — balans + W&V, `--format pdf` = the **KVK deposit package**; xlsx for the accountant; deprecated alias `jaarrekening report`) |
| `icp readout --period YYYY-Qn` | **ICP listing**: EU reverse-charge (verlegde) supplies per customer (from RE invoice lines), with their VAT ids. Fails `ICP_VAT_ID_MISSING` if a customer lacks one. Credit notes reduce the customer total |

```bash
bukio year-end status --year 2026
bukio year-end close --year 2026 --dry-run     # plan: result 1254.15 + postings
bukio year-end close --year 2026               # entries #9 #10 posted
bukio financial-statements report --year 2026 --model klein   # JSON
bukio financial-statements report --year 2026 --model klein --format pdf   # financial-statements-2026-klein.pdf (KVK)
bukio icp readout --period 2026-Q3             # EU customers + amounts
```

**OB readout fields (Phase 4 — the NL statutory VAT-return shape):** 1a/1b/1c omzet (21%/9%/0%-vrijgesteld), 1d privégebruik (21% auto-computed on `@P`), **2a verlegde EU leveringen (RE)**, 3a inkopen binnenland (incl. verlegd `@R`), **3b inkopen EU (RE)**, 3c buiten EU, 4a/4b verlegde btw, 5a verschuldigd, 5b voorbelasting, 5d te betalen/te ontvangen. 2b and 5c are not tracked.

### `bukio mcp` / `bukio fx` / `bukio compliance`

The agent layer (Phase 5).

| Command | Purpose |
|---------|---------|
| `mcp` | **MCP server over stdio** (JSON-RPC 2.0, newline-delimited): `company_info`, `trial_balance`, `balance_sheet`, `pnl`, `journal`, `accounts`, `vat_readout`, `icp_readout`, `audit`, `compliance`, `invoices` (read-only) + `entry_add/post/reverse`, `vat_book`, `invoice_create/finalize/credit/pay`, `recurring_run`, `year_end_close`, `fx_set`, `contact_add` (mutations). **Mutations are plan-only unless `mode:"execute"`**; `BUKIO_MCP_READONLY=1` blocks execution entirely. Every execute books with the caller's `actor` and lands in the audit log. NL query = an agent on top of these tools |
| `fx set --currency USD --date D --rate 1.0875 [--source text]` | Store a rate (1 EUR = N units of foreign currency, 4 decimals max; default source `manual`). Upsert; audited |
| `fx fetch --currency USD [--date D]` | **Fetch the ECB reference rate** (free, no key) for a currency on/before a date and store it (source `ECB`). Weekends/holidays fall back to the last business day; unknown currency or pre-1999 → `ECB_RATE_NOT_AVAILABLE` |
| `fx list [--limit N]` / `fx show --currency USD [--limit N]` | Rate store inspection (all currencies, or one currency's history) |
| `entry add / vat book --currency USD [--rate R]` | **Foreign-currency purchase invoices**: spec amounts are in the foreign currency, converted to EUR (round-half-up) at booking; the rate is auto-looked-up (exact date, else latest on/before) when `--rate` is omitted. **Missing rates are fetched live from the ECB** and stored for reuse — one network call ever per currency/date. The ledger stores EUR; each posting keeps `fx_currency`/`fx_amount_cents` (the original amount) — reversals negate both. VAT legs are computed on the EUR amounts. `BUKIO_FX_NO_FETCH=1` disables the network fallback (offline/air-gapped use) |
| `compliance status --year YYYY` | The profile's compliance calendar (NL: OB + ICP quarterly deadlines and the jaarrekening deposit, 13 months after FY end, art. 2:394 BW) with filed/open/overdue status; `compliance mark --type ICP\|JAARREKENING --period ...` records a filing (OB uses `vat readout --mark-filed`) |

### `bukio server` — remote access

Host the books on one machine (an in-house server, a VPS) and drive them from
anywhere — a phone, a laptop, another agent on a different VPS — while the
private signing keys stay on the devices that own them.

| Command | Purpose |
|---------|---------|
| `server start --listen <host:port> [--serve-db <path>] [--tls-cert C --tls-key K]` | Serve **ONE company DB** over HTTP(S). Clients POST signed command envelopes to `/rpc`; the server verifies each against the company registry (Ed25519 signature, ±5 min window, nonce replay refusal, authz) and runs it — output is **byte-identical to local mode** and audit rows carry the real signature (`audit verify` works server-side). Like `mcp`, the daemon itself is a bridge and needs no signature. Default `127.0.0.1:8787` — bind to a trusted network (localhost, a Tailscale IP) or use `--tls-cert/--tls-key` |
| `server token <actor> [--ttl-hours N]` | Mint a **one-time enrolment token** (single-use, TTL, actor-bound, stored sha256-hashed). Operator act on the server machine |
| `actor register --server <url> --token <t>` | Remote enrolment: the client's **public key** is enrolled; the private key never leaves the client. The token replaces the local enforce-off/register/enforce-on dance for remote first enrolment |
| `--server <url>` (global) or `BUKIO_SERVER` | Run ANY command remotely: `bukio --server https://bukio.intranet entry add ...` — sign locally, verify + execute on the server |

Local-only commands (`server *`, `mcp`, `init`, `update`, `actor
keygen/unlock/lock`) refuse with `LOCAL_ONLY` under `--server`. Same-device
use works too: point `--server` at `http://127.0.0.1:PORT` on the same host.
File outputs (PDF/XLSX exports, attachments) land on the server host — that
is where the books live.

#### Transport options (how the client reaches the server)

The envelope signature is the security layer that matters (identity,
integrity, replay). The transport's job is to keep the bytes private and to
decide who may reach the port at all. Three supported ways, in order of
preference:

1. **SSH tunnel (recommended for humans).** The daemon binds to loopback
   only (`--listen 127.0.0.1:8787` — the default — exposes nothing); the
   client opens a tunnel and points `--server` at the local end:

   ```bash
   # server: bukio server start --listen 127.0.0.1:8787 --serve-db /srv/bukio/company.db --actor agent:op
   # client: forward the port, then use it like any remote
   ssh -N -L 8787:127.0.0.1:8787 user@server
   bukio --server http://127.0.0.1:8787 report trial-balance --actor human:erik
   ```

   You get SSH's battle-tested encryption, host verification and key-based
   access control, with **zero new listening ports** on the server. Access
   control = who has an SSH key; every command is still signed by the
   client's own actor key, so the audit trail keeps per-device attribution.
   No TLS certificates to manage.

2. **Tailscale / WireGuard (recommended for agents & always-on clients).**
   Both machines join a private network (tailnet); the daemon binds to the
   server's tailnet IP (`--listen 100.x.y.z:8787`) and is reachable only by
   other tailnet members. Non-interactive clients (cron, agents on another
   VPS) get a stable endpoint without needing an SSH session per call, and
   the overlay encrypts everything in transit (mTLS by default on
   Tailscale). Access control = the tailnet ACL.

3. **Native TLS (direct exposure).** If the daemon must listen on a public
   interface, serve HTTPS with `--tls-cert C --tls-key K` (self-signed is
   fine — pin it on the clients) so the wire is encrypted. Access control
   then rests on the envelope signature alone: anyone who can reach the port
   can *try* commands, but only enrolled actors' signed commands execute
   (and `actor enforce on` refuses everything unsigned).

**Do not** run plain HTTP on a public interface — signatures prove *who
signed* and that nothing was *altered*, but they do not hide the command
contents from eavesdroppers. Plain HTTP is only safe on a network you
already trust (loopback, a tailnet, a LAN).

**Why not `ssh user@server "bukio …"` instead?** It works, but the command
then runs with the *server-side* actor's key — every remote user collapses
into one server identity, and the audit row no longer proves *which person
or agent* on which device acted. That is the accountability problem the
signing model exists to solve. SSH as the *transport* (option 1) gives you
SSH's security **and** per-device attribution.

### `bukio import` / `bukio month-end` / `bukio invoice reminders`

Imports & period automation (Phase 6).

**Every importer validates the ENTIRE file before writing anything** — all
errors are collected and reported with line numbers (`IMPORT_VALIDATION_FAILED`
+ `details`), and the file is rejected as a whole when anything is wrong.
Imports are idempotent: re-running skips already-imported boekstukken.

| Command | Purpose |
|---------|---------|
| `import opening-balances --file <csv> [--date yyyy-mm-dd] [--dry-run]` | Import opening balances as ONE posted `Beginbalans` entry. CSV: `code,amount` (signed; + = debet) or `code,debet,credit` (Dutch layout). Amounts accept `1234.56`, `1234,56` and `1.234,56`. Sum must be zero. Re-import → `OPENING_ALREADY_IMPORTED` |
| `import invoice --file <ubl.xml> [--contact N] [--create-missing] [--dry-run]` | **Inbound e-invoice (v0.14)**: parse an EN 16931/Peppol BIS 3.0 UBL invoice (fast-xml-parser) into the **payables register** — whole-file validation, supplier auto-created with `--create-missing` (matched by btw-id → name → explicit `--contact`), idempotent by supplier+invoice number, VAT breakdown per rate reported but **not booked** (book via the normal workflow). Credit notes (type 381) → `UNSUPPORTED_UBL_DOCUMENT` |
| `import journal --file <csv> [--create-missing] [--dry-run]` | Import a journal from SnelStart/Exact-style CSV: header `datum,boekstuknummer,rekening,tegenrekening,bedrag[,omschrijving][,btwcode]` (aliases + `;` delimiter supported). Each row books `+bedrag` on rekening / `−bedrag` on tegenrekening; rows per boekstuknummer become ONE posted entry (`source='import'`, `source_ref='journal:<nr>'`). `--create-missing` creates unknown accounts (type inferred from net movement). BtwCode columns are reported in `ignored_btw_codes` but not booked |
| `import xaf --file <audit.xaf> [--dry-run]` | Import an **XML Auditfile Financieel 4.0** — both the Belastingdienst layout (`Xaf`/`Rekeningen`/`Mutaties`) and the generic AuditFile layout (`AuditFile`/`Header`/`MasterFiles`/`GeneralLedgerEntries`, explicit debit/credit lines): the file's chart is upserted (types from `AccountType`/`RekeningSoort`), each Mutatie/Transaction becomes one posted entry (`source='xaf'`, `source_ref=<boekstuk/transaction>`). A differing company KVK → `COMPANY_MISMATCH` (name differences are warnings). **On an empty ledger the file's chart is authoritative**: colliding account codes are renamed to the file's meanings (listed in the dry-run as `accounts_to_rename`, reported as `accounts_updated` after import); accounts that already carry postings are never touched |
| `month-end --period yyyy-mm` | **Read-only close check**: draft entries, unmatched bank transactions, the VAT-return readout for the containing quarter, draft + overdue invoices (with outstanding total), due recurring templates, period debit/credit totals (`balanced`), the month's profit, and human-readable `warnings` — the agent's monthly "can I close?" report |
| `import contacts --file <audit.xaf> [--dry-run]` | Import **suppliers + customers** from an audit file (either XAF layout) as invoice contacts: name, street, postal code, city, country, email, vat-id. Whole-file validation (every entry needs a name); idempotent by name |
| `invoice reminders [--within-days N] [--draft-emails]` | Overdue + due-soon sales invoices, sorted most-overdue first, with `outstanding` per invoice. `--draft-emails` adds a localized reminder email draft (`to`/`subject`/`body`, per invoice language) per invoice — nothing is ever sent |
| `assets scheme add --name [--method lineair\|degressief] [--life-months 60] [--residual-bp 0]` | Create a depreciation scheme. Default scheme (created lazily): **5 years lineair, monthly, 0% residual** |
| `assets add --name --purchase-date --purchase-price --depreciation-start --recognition-date [--cum-dep] [--scheme] [--asset-account 1800] [--cum-dep-account] [--expense-account 4600] [--entry-id] [--category] [--serial] [--residual]` | Register an **already-booked** asset: only the *remaining* depreciation is booked from the recognition date (first run on the 1st). GL reconciliation warnings, never blockers |
| `assets run [--period yyyy-mm \| --as-of DATE] [--dry-run]` | Book due depreciation runs — `source='assets'`, idempotent per asset-month, auto-completes assets at the residual |
| `assets register [--as-of DATE] [--format json\|csv\|xlsx --out]` | The **asset register (activastaat)**: cost, cumulative depreciation, book value per asset + totals |
| `assets dispose --id N --date [--proceeds] [--bank-account 1100] [--result-account 8100] [--dry-run]` | Dispose (sale or scrap): proposes the full entry (bank / cum-dep / asset / winst-verlies), status → `disposed` |
| `assets list [--status]` / `show --id` / `pause --id` / `resume --id` | Register inspection + depreciation pause/resume |
| `payments payables add --contact N --ref --date --amount [--due] [--method transfer\|direct-debit] [--entry-id]` | Register a purchase invoice (payable). `direct-debit` = incasso (collected by the vendor — only joins direct-debit batches) | 
| `payments payables list [--status] [--method]` / `pay --id` | Open payables (unpaid / in_batch / paid); mark paid after the bank statement confirms |
| `payments mandate add --contact N --ref R [--date D] [--type core\|b2b]` / `list [--contact]` / `remove --id` | **Incassovolmacht register (v0.14)**: SEPA mandates per contact. `core` = 8-week refund right, `b2b` = none. Required before a direct-debit batch can collect from a contact (`MANDATE_REQUIRED` otherwise) |
| `payments batch create [--type transfer\|direct-debit] [--from-invoices] [--payable ids] [--lines "C:AMT[:REF];…"] [--csv file] [--date] [--from-iban] [--dry-run]` | Build a batch from unpaid payables matching the type and/or explicit lines; whole-set validation (IBAN mod-97, amounts, refs). Direct-debit lines auto-carry the contact's mandate snapshot + FRST/RCUR sequence |
| `payments batch export --id N [--schema 001.03\|001.09] [--out file.xml] [--dry-run]` | Export SEPA XML for bank-portal upload: **pain.001** for transfer batches, **pain.008.001.02** for direct-debit (one `PmtInf` per CORE/B2B scheme). Once per batch (unique `MsgId` — re-export would double-pay/collect) |
| `payments batch list [--status]` / `show --id` / `delete --id` | Batch tracking; delete only allowed on drafts (releases payables back to unpaid) |
| `contact update --id N [--iban] [--address] …` / `contact add --iban` | Contact IBANs (mod-97 validated) — required to include a vendor in a batch |
| `export xaf --year yyyy --out <file.xaf>` | Export the fiscal year as an **Auditfile Financieel 4.0 XML** (Belastingdienst standard) — the file a boekhouder, tax advisor or auditor imports directly into SnelStart/Exact. One `<Mutatie>` per **posted** entry (drafts excluded); postings as `<Boeking>` debet/credit pairs that re-import losslessly. Records an `export.xaf` audit row; read-only |

```bash
# switching from your old package in one morning:
bukio import opening-balances --file beginbalans.csv --date 2026-01-01
bukio import journal --file snelstart-export.csv --create-missing
bukio import xaf --file audit.xaf                      # the Belastingdienst format
# let an agent run the close every month:
bukio month-end --period 2026-08
bukio invoice reminders --within-days 7 --draft-emails
# hand the year to your boekhouder / tax advisor / auditor:
bukio export xaf --year 2026 --out ~/exports/bukio-2026.xaf
bukio audit --format xlsx --out ~/exports/bukio-audit-2026.xlsx --limit 1000
```

**Import validation notes:** amounts accept the international form (`1234.56`)
and Dutch bookkeeping notation (`1234,56`, `1.234,56`); `;`-delimited files
split on `;` (decimal commas stay intact), otherwise on `,`. Journal lines
without a boekstuknummer, rows on different dates within one boekstuk, unknown
accounts (without `--create-missing`), and unbalanced opening balances all
reject the file with per-line `details`.

```bash
bukio fx set --currency USD --date 2026-07-03 --rate 1.0875
bukio fx fetch --currency GBP --date 2026-08-03          # ECB reference rate, stored
bukio vat book --date 2026-08-01 --desc "Stripe (USD)" --currency USD \
  --postings "4300:895.00@21,1100:-1082.95" --post      # 779.28 EUR — rate auto-fetched from the ECB
# koersverschil at payment: book the difference on 4700 (created on demand)
bukio account add --code 4700 --name "Koersverschillen" --type expense --normal-balance debit
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"entry_add","arguments":{"date":"2026-07-31","description":"Huur","postings":["4300:800.00","1100:-800.00"],"mode":"execute","actor":"agent:hermes"}}}' \
| bukio mcp           # or wire it into an MCP client (Hermes, Claude Code, ...)
bukio compliance status --year 2026
```

**FX booking rules:** amounts in posting specs are foreign currency; the rate
resolves as `--rate` → stored rate (exact, else latest on/before) → **ECB
reference rate** (fetched live, stored as source `ECB` for reuse). `--rate`
always wins; `BUKIO_FX_NO_FETCH=1` keeps bukio fully offline. The description
should note the currency and the original invoice number. Outgoing invoices
stay EUR-only in the shipped markets — the invoice-compliance and UBL paths
are EUR-based, and the non-EUR markets (GB/US/…) cannot finalize invoices
yet (no art. 226 EU baseline for them; the twelve EU markets all finalize
via the harmonized baseline + their localised PDFs).

### `bukio backup` / `bukio restore` / `bukio attach`

| Command | Purpose |
|---------|---------|
| `backup [--out <path>] [--encrypt] [--passphrase] [--keep N] [--dry-run]` | Consistent SQLite backup (default `~/.bukio/backups/bukio-<ts>.db`). **`--encrypt` (v0.14)** wraps it in AES-256-GCM (scrypt-derived key) — file extension `.enc`, passphrase from `--passphrase` or `BUKIO_BACKUP_PASSPHRASE` env (never in the repo). **`--keep N`** prunes the oldest backups in the default folder (rejects `--out` — rotation only applies to the default location) |
| `restore --from <file> [--to <path>] [--force] [--passphrase]` | Restore from a backup file (validated first); **encrypted backups are auto-detected** by the `BUKIOENC1` magic header and decrypted with `--passphrase` / `BUKIO_BACKUP_PASSPHRASE` |
| `update [--yes] [--repo <path>] [--trust-remote] [--dry-run]` | **Self-update** from the GitHub main branch: fetch `origin/main` and reset the working tree to it (audit row when a company DB exists; works without one). ⚠️ The reset **overwrites local customizations** — `--dry-run` first (incoming commits + modified files + local commits that would be lost), and the real run refuses without `--yes`. `--trust-remote` for forks/mirrors, `--repo` for a different install, npm installs update with `npm update -g bukio-cli` |
| `attach add --invoice N \| --entry N --file F [--store db\|file] [--note] [--dry-run]` | **Source documents (v0.14)**: store the original PDF/scans against an invoice or entry. Default `--store db` = BLOB in the SQLite file (travels with backups; 25 MB/file cap; sha256 dedupe). `--store file` = content-addressed copy in `<db>-attachments/` |
| `attach list --invoice N \| --entry N` / `show --id [--out F]` / `remove --id` | Metadata-only listing (never reads the BLOB); `show` extracts the bytes (`--force` to overwrite); `remove` deletes the BLOB/copy. Add/remove are audited |

`restore` refuses to overwrite an existing initialised database unless `--force` is given, and refuses `--from`/`--to` pointing at the same file. Wrong passphrase → `BACKUP_PASSPHRASE_WRONG` (tamper-proof via GCM auth tag).

```bash
bukio backup                                   # ~/.bukio/backups/bukio-<ts>.db
BUKIO_BACKUP_PASSPHRASE='...' bukio backup --encrypt --keep 30
bukio restore --from ~/.bukio/backups/bukio-<ts>.db.enc --to ~/.bukio/restored.db
bukio attach add --invoice 1 --file ~/invoices/2026-08-01_acme_F2026-123.pdf
bukio attach list --invoice 1
```

### `bukio audit`

Read the append-only audit log (newest first).

| Option | Default | Description |
|--------|---------|-------------|
| `--since <iso-ts>` | — | Only entries at/after this timestamp (ISO 8601) |
| `--by <who>` | all | Only entries by this actor (e.g. `agent:bartholomeus`) |
| `--limit <n>` | `50` | Max rows |
| `--format <fmt>` | `human` | `json` \| `csv` \| `xlsx` — hand the audit trail to an external advisor as a file |
| `--out <path>` | — | Output file for `csv`/`xlsx` (required for `xlsx`) |

```bash
bukio audit --by agent:bartholomeus --json   # what did the agent do?
bukio audit --since 2026-08-01         # everything this month
bukio audit --format xlsx --out ~/exports/audit-2026.xlsx --limit 1000   # for the boekhouder
```

### `bukio audit verify`

Re-verify the signed audit trail against the company's key registry. For every row, the canonical digest is recomputed from the stored signed args and the signature is re-checked. Reads only the database — a **copied** DB file verifies self-contained, with no key files present.

Each row gets one of: `ok` · `unsigned` (legacy/pre-signing rows — not an error) · `revoked` (signature verifies, key since revoked — valid at the time) · `tampered` (args no longer produce the signed digest) · `invalid-signature` · `unknown-key`. The summary prints counts; **exit code is 1** when tampered/invalid/unknown-key rows exist (handy for scheduled checks).

| Option | Default | Description |
|--------|---------|-------------|
| `--since <iso-ts>` | — | Only rows at/after this timestamp |
| `--limit <n>` | all | Check only the newest N rows |

```bash
bukio audit verify --json        # full report
bukio audit verify               # summary + problem rows
```

### `bukio actor`

Key management for actor identity & signing (see [Actor identity & signing](#actor-identity--signing)). All `actor` commands are **exempt from signing** — they are the bootstrap that makes signing possible.

| Command | Description |
|---------|-------------|
| `actor keygen [--force] [--dry-run]` | Generate an Ed25519 keypair for the `--actor`: human keys are passphrase-encrypted (`BUKIO_SIGNING_PASSPHRASE` or interactive prompt), agent/system keys are plain files. `--force` replaces an existing key (rotation). |
| `actor register [--dry-run]` | Enrol the actor's local key into the **current company's** DB. Per-company: repeat for every company DB the actor works in. **First enrolment is refused under enforcement** — onboard with `enforce --off` → `register` → `--on`; re-enrolment after revocation (rotation) works under enforcement. |
| `actor list` | List enrolled keys in the current company's DB — active and revoked (with reason), full history. |
| `actor revoke --reason <text> [--target <who>] [--dry-run]` | Revoke an actor's key in the current company's DB (the row is retained as history). Default: your own key. `--target <who>` is the **owner-mediated kill** of a compromised key — owner role required **regardless** of authz mode. |
| `actor enforce --on \| --off` | Turn signature enforcement on/off for the current company's DB. **Only an enrolled actor can turn it off** (audited; the deliberate recovery valve — see the enforcement bullet above). |
| `actor authz --on \| --off [--dry-run]` | Toggle **per-actor authorizations** (segregation of duties). `--on` implies signing enforcement and grants the flipper the `owner` role (bootstrap); `--off` needs the owner role (enforcement stays on). |
| `actor roles [--for <who>]` / `roles grant <role> --for <who>` / `roles revoke <role> --for <who>` | Role grants (owner only under authz; listing your own roles is self-service). Grants warn softly on segregation-of-duties conflicts. The LAST owner can never be revoked. |
| `actor can '<cmd>' [--for <who>]` | Capability check — `ok` or `AUTHZ_DENIED` with the missing capability. Self-service for your own checks; `--for <who>` is owner-only. Accepts `entry add --post` and `mcp:entry_add`. |
| `actor who-can '<cmd>'` | Which actors can run a command — the SoD review matrix (owner only under authz). |
| `actor unlock [--ttl-hours <n>]` | Human keys only: decrypt the passphrase-protected key into a short-lived session key (default 12 h, max 168). |
| `actor lock` | Clear the session key — the human must authenticate again. |
| `actor verify` | Show the actor's key state: key file present? enrolled? session valid? enforcement? |

```bash
bukio actor keygen                       # as agent:bartholomeus — plain key file
BUKIO_SIGNING_PASSPHRASE=… bukio actor keygen   # as human:erik — encrypted
bukio actor unlock --ttl-hours 12        # human session
bukio actor register                     # enrol into THIS company's DB
bukio actor enforce --on                 # from now on: unsigned = refused
bukio actor authz --on                   # authorizations: roles gate every command
bukio actor roles grant bookkeeper --for agent:invoicing
bukio actor who-can 'entry post'         # the SoD review matrix
```

---

## Global Flags

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--json` | — | off | Machine-readable JSON output (see below) |
| `--locale <code>` | `BUKIO_LOCALE` | `en` | Output language for human-facing text (see [Localization](#localization-i18n)) |
| `--db <path>` | `BUKIO_DB` | `~/.bukio/bukio.db` | Database file |
| `--actor <who>` | `BUKIO_ACTOR` | *(required)* | Acting entity — `'<role>:<name>'`, e.g. `agent:bartholomeus`, `human:erik` |
| `--sign-key <path>` | — | actor session → `BUKIO_SIGNING_PASSPHRASE` → actor key file | Explicit private key to sign with; by default the signing key resolves in that order |
| — | `BUKIO_SIGNING_PASSPHRASE` | — | Passphrase for human (encrypted) keys — used by keygen/unlock and by the sign gate when no session exists |
| config dir | `BUKIO_CONFIG_DIR` | `~/.bukio` | Where keys (`keys/`) and sessions (`sessions/`) live |

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

## Localization (i18n)

Human-facing output is **English by default**. Localization is optional and opt-in:

- `--locale <code>` (global flag) or `BUKIO_LOCALE` env — the output language for human-facing text.
- Unknown locales and disabled localization fall back to **English**; regional codes resolve to their base language (e.g. `nl-BE` → Belgian-Dutch table → Dutch → English).
- Locale tables: `en` (default), `nl`, `nl-be`, `de`, `fr`, `fr-lu`, `da`, `fi`, `nb`, `sv` — covering all eleven markets.

**What localizes:** invoice PDF labels and units, invoice + reminder emails, CLI table headers and renders (invoice list, reminders, P&L, balance sheet, month-end, year-end), VAT file/settle descriptions, status and direction labels.

**What never localizes:** the `--json` contract (keys, messages, amounts stay English), error codes, MCP tool names/descriptions, chart and account identifiers, and statutory artifacts — the OB readout labels (fields 1a–5d), the statutory jaarrekening XLSX sheet ('Winst en verlies'), statutory model labels ('Onverdeeld resultaat'), RGS group names, command aliases.

**Document language** (`invoice create --language nl|en`): the PDF/email language follows the document's own language field; when omitted it follows the company profile — Dutch for NL/BE companies, English for every other market (no market is the de facto base). Per-locale `pdf.*` table keys are reserved for document-language expansion.

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
2. **Always pass `--actor '<role>:<name>'`** (e.g. `agent:bartholomeus`, `human:erik`) so the audit trail attributes your work — it is required.
3. **Prefer `--json`** for parsing; keep human-readable output for humans.
4. **Never edit the SQLite file directly.** Use the CLI/engine — the triggers and audit log exist for a reason.
5. **Never delete a posted entry.** Reverse it.
6. **Verify after every mutation** (e.g. `report trial-balance --json` must say `balanced: true`).

---

## Scheduling recurring actions (cron)

bukio never runs itself — the schedule engine, the asset module and the close
check only act when someone calls them. That is exactly what makes them good
cron jobs. Two flavours:

- **Read-only jobs** (reminders, deadline calendar, close check, dry-run plans)
  — safe to run unattended; output lands in a log.
- **Mutating jobs** (`recurring run`, `assets run`) — they book entries, so
  always **dry-run first**. The recommended pattern is an agent-driven cron
  (e.g. Hermes Agent): produce the plan → verify → apply → re-verify
  `trial-balance` → backup. Never let a bare cron book blindly.

### Recommended schedule

| Cadence | Command | Kind |
|---|---|---|
| Daily | `invoice reminders --within-days 7 --draft-emails --json` | read-only |
| Weekly | `compliance status --year <yyyy> --json` | read-only |
| Weekly | `backup --out …` + `tar` the invoice archive | backup |
| Monthly (1st) | `recurring run --as-of <1st> --dry-run --json` | plan |
| Monthly (1st) | `assets run --period <prev> --dry-run --json` | plan |
| Monthly (1st) | `month-end --period <prev> --json` | read-only |
| Quarterly | `vat readout --period <yyyy-Qn> --json` | read-only |

`recurring run` and `assets run` are idempotent and backfill missed periods —
if a cron tick was missed (server down), the next run simply catches up.

### Plain system crontab (read-only + backup — safe unattended)

```cron
# ── daily 08:00 — overdue/due-soon invoices (draft emails only, never sends)
0 8 * * *  BUKIO_DB=~/.bukio/bukio.db bukio invoice reminders --within-days 7 --draft-emails --json >> ~/.bukio/cron/invoices.log 2>&1

# ── weekly Mon 08:30 — filing-deadline calendar
30 8 * * 1 BUKIO_DB=~/.bukio/bukio.db bukio compliance status --year $(date +\%Y) --json >> ~/.bukio/cron/compliance.log 2>&1

# ── weekly Sun 07:00 — consistent DB snapshot + document archive
0 7 * * 0  BUKIO_DB=~/.bukio/bukio.db bukio backup --out ~/.bukio/backups/bukio-$(date +\%F).db >> ~/.bukio/cron/backup.log 2>&1
0 7 * * 0  tar -czf ~/.bukio/backups/invoices-$(date +\%F).tar.gz -C ~/.bukio invoices >> ~/.bukio/cron/backup.log 2>&1

# ── 1st of month 09:00 — the close check (read-only)
0 9 1 * *  BUKIO_DB=~/.bukio/bukio.db bukio month-end --period $(date -d "1 month ago" +\%Y-\%m) --json >> ~/.bukio/cron/month-end.log 2>&1
```

The mutating pair (`recurring run`, `assets run`) deliberately has **no
unattended line here** — their dry-run plans belong in the agent-driven loop
below, where a human or agent reviews before anything is posted.

### Agent-driven month-end loop (mutating — plan, verify, apply)

With an agentic harness (e.g. Hermes Agent), the monthly close becomes one
reviewed run instead of blind cron lines. Suggested job prompt:

```text
Run the bukio month-end for <prev-month>:
1. bukio recurring run --as-of <1st> --dry-run --json   → show the plan
2. bukio assets run --period <prev-month> --dry-run --json → show the plan
3. after approval: apply both without --dry-run (--actor agent:<name>)
4. bukio report trial-balance --json                    → must be balanced: true
5. bukio month-end --period <prev-month> --json         → all clear?
6. bukio backup --out ~/.bukio/backups/bukio-<date>.db + tar the invoice archive
```

Never skip the dry-run step; the whole point of bukio's `--dry-run` is that a
machine can propose and a human (or a verifying agent) disposes.

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

The version history is recorded in [CHANGELOG.md](CHANGELOG.md); the agent manual for driving the tool lives in [AGENTS.md](AGENTS.md).

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
| `INVALID_SOURCE` | Unknown source (`manual`/`bank`/`invoice`/`agent`/`reversal`/`recurring`/`closing`/`import`/`xaf`/`assets`) |
| `INVALID_ACTOR` | Actor is empty |
| `TOO_FEW_POSTINGS` | Fewer than 2 postings |
| `UNBALANCED` | Postings do not sum to zero |
| `ASSET_NOT_FOUND` / `SCHEME_NOT_FOUND` | Asset / scheme does not exist |
| `ALREADY_DISPOSED` / `INVALID_STATUS` | Asset already disposed / wrong status for pause-resume |
| `COMPANY_INCOMPLETE` | Company has no valid IBAN — set one with `company update --iban` (needed for batches) |
| `CONTACT_IBAN_MISSING` | Contact has no IBAN — `contact update --id N --iban` |
| `BATCH_VALIDATION_FAILED` | Batch lines failed validation (per-line `details`) |
| `BATCH_ALREADY_EXPORTED` | Batch already exported — exporting again could double-pay; create a new batch |
| `PAYABLE_DIRECT_DEBIT` / `PAYABLE_NOT_UNPAID` | Payable excluded from batches (incasso) / not in `unpaid` state |
| `INVALID_LIFE` / `INVALID_METHOD` / `SCHEME_NAME_TAKEN` | Scheme validation (life 1-600 months, method lineair\|degressief, unique name) |
| `INVALID_DEPRECIATION` | Cumulative depreciation at recognition exceeds cost minus residual |
| `INVALID_COST` / `INVALID_RESIDUAL` | Asset purchase price / residual value invalid |
| `ENTRY_NOT_FOUND` | The `--entry-id` purchase-booking link does not exist |
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
| `INVALID_FREQUENCY` / `INVALID_DATE` / `INVALID_RANGE` | Recurring template schedule invalid |
| `INVALID_RUNS` / `INVALID_COST` / `INVALID_RESIDUAL` / `INVALID_LIFE` | Depreciation parameters invalid |
| `ALREADY_COMPLETED` | A completed recurring template cannot be re-activated |
| `RECURRING_ERROR` | A template failed during `recurring run` (reported per-template, others continue) |
| `SUPPLIER_INCOMPLETE` / `CUSTOMER_INCOMPLETE` | Invoice missing supplier/customer vereisten — set them at `init` / `contact add` |
| `CUSTOMER_VAT_REQUIRED` | reverse-charge line needs the customer's VAT id |
| `INVALID_LINE` / `NO_LINES` / `CONTACT_NOT_FOUND` | Invoice line/contact validation |
| `ALREADY_FINALIZED` / `NOT_FINALIZED` | Invoice lifecycle violations |
| `OVERPAYMENT` / `NOT_PAYABLE` / `CREDIT_NOT_PAYABLE` | Payment validation |
| `PDF_UNAVAILABLE` | Playwright/Chromium could not render the invoice PDF |
| `PEPPOL_NOT_CONFIGURED` / `PEPPOL_SEND_FAILED` | Peppol provider missing (env `BUKIO_PEPPOL_ENDPOINT`) or rejected the document |
| `INVALID_KIND` / `INVALID_REVERSE` | Recurring template kind errors (reverse-previous is entry-only) |
| `INCOMPLETE_YEAR` / `ALREADY_CLOSED` / `EMPTY_YEAR` / `INVALID_YEAR` | Year-end close guards |
| `INVALID_MODEL` | jaarrekening model must be micro or klein |
| `ICP_VAT_ID_MISSING` | EU customer without a btw-id — the ICP listing cannot be completed |
| `FX_RATE_NOT_FOUND` / `INVALID_RATE` / `INVALID_CURRENCY` / `INVALID_FX_AMOUNT` / `INVALID_FX_CURRENCY` | FX booking errors (missing rate, malformed rate/currency/amount) |
| `ECB_FETCH_FAILED` / `ECB_RATE_NOT_AVAILABLE` | ECB unreachable, or no reference rate for the currency/date (unknown currency, pre-1999) |
| `MCP_READONLY` | A mutation was attempted on a read-only MCP server (BUKIO_MCP_READONLY=1) |
| `INVALID_TYPE` / `INVALID_PERIOD` | compliance mark errors |
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
bukio report balance-sheet --as-of 2026-12-31   # must say BALANCED
bukio report pnl --year 2026                    # result = revenue - costs
bukio audit --since 2026-08-01 --by agent:bartholomeus
```

**Hand the year to your boekhouder**
```bash
bukio report journal --year 2026 --format xlsx --out ~/exports/journal-2026.xlsx
bukio report balance-sheet --as-of 2026-12-31 --format csv --out ~/exports/balance-sheet-2026.csv
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

## EU AI Act Transparency

*Regulation (EU) 2024/1689 — the EU Artificial Intelligence Act.*

**This software is not an AI system.** bukio-cli is deterministic, rule-based
accounting software: every booking, VAT calculation and report follows fixed
double-entry rules over integer cents. It performs no inference, no
machine learning, no autonomous decision-making, and no profiling — so the
obligations the AI Act places on providers/deployers of AI systems (high-risk
requirements, conformity assessment, risk management, Article 50 interaction
transparency) do not apply to the product itself.

**The code, however, was written with AI assistance.** This section is the
project's transparency disclosure, in the spirit of the Act's transparency
principle for AI-generated content:

| Aspect | Disclosure |
|---|---|
| Development method | All source, tests and documentation were generated with an AI coding assistant (Hermes Agent, running `deepseek-v4-flash`), then reviewed, verified and accepted by the repository owner. |
| Human oversight | Every commit is reviewed by the owner before it lands; the automated test suite (495 tests, `npm test`) must pass; money paths additionally require a balanced trial balance. Nothing is accepted blind. |
| Synthetic content | Code, tests and docs are AI-generated output; this README section and the commit history serve as the disclosure that the content is machine-generated. |
| Model provider obligations | The underlying general-purpose AI model is provided by DeepSeek; its obligations under the AI Act (e.g. Article 53 documentation, copyright policy, training-data summary) sit with the provider, not with this repository. |
| No prohibited practices | The project involves none of the Article 5 prohibited practices (no social scoring, no biometric identification, no manipulation). |
| No high-risk use | Bookkeeping is not a high-risk use case under Annex III; no fundamental-rights decisions are made by this software. |
| AI literacy | The developer maintains AI literacy (Article 4) and exercises it: every AI output is verified against accounting invariants before use. |

Status for the record: the AI Act entered into force on 1 August 2024;
prohibitions and AI-literacy obligations applied from 2 February 2025; GPAI
and governance provisions from 2 August 2025; the remainder of the Act applies
from 2 August 2026. This disclosure is provided as a matter of transparency
and good faith; it is not legal advice.

---

## AI Development Cost & Token Usage

The entire project was built with AI assistance. For full transparency, here
is the measured token consumption and its cost at **official list prices**
(per 1M tokens; OpenCode Go / DeepSeek API, Aug 2026): **DeepSeek V4 Flash**
**$0.14** input (cache miss), **$0.0028** cached input, **$0.28** output;
**MiMo-V2.5-Pro** **$0.435** input, **$0.003625** cached input, **$0.87**
output. Reasoning tokens are billed at the output rate; other models
(minimax-m2.5) are priced at the OpenCode Go list rate. Data is captured by
the `bukio-token-track` tool from the agent's session telemetry — including
delegation subagent sessions, bukio-cli sessions only (snapshot 2026-08-15).

**Proven stack:** bukio-cli is developed and operated end-to-end with
**Hermes Agent** (Nous Research) via OpenCode Go. The main development
sessions ran **DeepSeek V4 Flash** (a handful of calls via the DeepSeek
API and other providers directly); the parallel code-review subagents (delegation batches) ran
**MiMo-V2.5-Pro**, also via OpenCode Go. The live day-to-day operations
(bank imports, invoice booking, month-end checks) run on the same stack
against this same codebase.

### Token usage — per model

| Model | API calls | Input | Cached input | Output | Reasoning | Est. cost |
|---|---|---|---|---|---|---|
| DeepSeek V4 Flash | 11,215 | 33.86M | 2,294.04M | 9.71M | 5.70M | $15.48 |
| MiMo-V2.5-Pro (review subagents) | 574 | 6.82M | 42.12M | 1.06M | — | $4.05 |
| Other models (minimax-m2.5, mimo-v2.5) | 29 | 0.19M | 0.80M | 0.02M | 0.01M | $0.04 |
| **Total** | **11,818** | **40.87M** | **2,336.96M** | **10.79M** | **5.71M** | **$19.56** |

**$19.56 total** at official list prices for the entire project (11,818
API calls across all development sessions, ≈ 2.39B tokens).

### Developer Time (contributed, unpaid)

Beyond API spend, this project took my review-and-direction time. Because
the agent does the building, my own messages are the only interaction
channel — so I measure my time by them: across the 24 working sessions
(Aug 4–15, 2026, plus 178 review-subagent sessions) I sent **996 messages**, each costed at **≈ 60 s of
overhead** (reading, deciding, reviewing) plus composition time scaled by
message length and complexity (≈ 2 min/message effective). That works out to
**≈ 33 hours total**, all contributed unpaid.

At a **senior** Dutch software-developer rate of **≈ €45/hour** (Amsterdam
senior average, 2026: €45/h
[Glassdoor](https://www.glassdoor.com/Salaries/amsterdam-netherlands-senior-software-engineer-salary-SRCH_IL.0,21_IM1112_KO22,46.htm),
€45.50/h
[SalaryExpert](https://www.salaryexpert.com/salary/job/software-developer/netherlands/amsterdam);
the national average is lower), my time is worth **≈ €1,490**.

Stated plainly, so nothing is hidden:

- **Compliance:** all of this work happened in my free time, outside working
  hours — no employer time, equipment, or other resources were used.
- **Deliberately conservative:** I am an amateur developer, and a senior
  professional rate overstates the market value of my review time by a wide
  margin. I include it high on purpose: every cost of this project is
  quantified rather than tucked away as unmeasured "effort and work".
- **It was free:** the ≈ €1,490 is an imputed opportunity cost, not money paid.
  My out-of-pocket spend remains **$19.56** in API costs.
- **Not a full review:** these hours do not come close to the effort a
  conventional code review of a 34.2 KLOC codebase would take; treat them as
  my direction-and-check time, not a substitute for professional review.

### COCOMO benchmark

For a frame of reference, the same codebase priced by the classic COCOMO
model (Boehm, 1981): **34,223 non-blank, non-comment lines of JavaScript**
across 138 files (19,139 in `src/`, 14,928 in `test/`, 156 in `bin/` +
`scripts/`), i.e. **34.22 KLOC**
(measured with `scc` v3.7.0).

| COCOMO mode | Effort (person-months) | Duration | Team size | Cost @ €9,000/PM\* |
|---|---|---|---|---|
| Organic | 98.0 PM | 14.3 months | ~7 developers | ≈ €882K |
| Semi-detached | 156.9 PM | 14.7 months | ~11 developers | ≈ €1,412K |
| Embedded | 249.7 PM | 14.6 months | ~17 developers | ≈ €2,247K |

\*Fully-loaded senior developer rate in the Netherlands (2026).

**Comparison:** a conventional team building this would estimate **≈ 98–250
person-months (≈ €882K–€2,247K)**; the AI-assisted build consumed **$19.56 in
API costs plus ≈ €1,490 of my review-and-direction time (contributed, unpaid
— see above)** over 24 working sessions in eleven days — still a tiny fraction of
the conventional estimate.
COCOMO is a rough 1981-era estimate (organic/semi-detached/embedded are the
three standard modes); treat the ratios, not the decimals, as the point.

---

## Troubleshooting

Ran into a question, bug, or anything else you need to know? **Ask your
agent first** — it has the full agent manual (AGENTS.md) and this README in
context. If your agent is unable to help, shoot me a message at
<erik@posthumanresources.nl> and I'll try to answer it when I'm able.

---

## Supported jurisdictions

Thirteen jurisdiction profiles (NL plus the twelve-market expansion; see AGENTS.md §3.1 for the full table). `bukio init --country <cc>` seeds the country's chart convention (RGS, PCN 2020, PCG, SKR 03, BAS 2023, NS 4102, …), VAT codes/rates, identifiers and compliance calendar. Format dispatch is strict: markets whose engines are B-milestones fail loudly (`FORMAT_NOT_SUPPORTED`) — no market ever silently gets another market's output.

**Parked** — CH Switzerland (CHF base currency, QR-bill, not a Peppol country).

| Country | Currency | VAT (2026) | Chart convention | Peppol scheme | Locale |
|---|---|---|---|---|---|
| NL Netherlands | EUR | 21 / 9 / 0 | RGS-mapped | 9944 (kvk) | nl |
| LU Luxembourg | EUR | 17 / 14 / 8 / 3 | PCN 2020 | 0195 (RCS) | fr (fr-lu) |
| GB United Kingdom | GBP | 20 / 5 / 0 | QuickBooks/Xero-style | — (2026 roadmap) | en |
| FR France | EUR | 20 / 10 / 5.5 / 2.1 | PCG (plan comptable général) | 0002 (SIREN) | fr |
| US United States | USD | no federal VAT | QuickBooks-style | — | en |
| BE Belgium | EUR | 21 / 12 / 6 / 0 | PCN-BE minimum plan (AR 12-09-1983) | 0208 (KBO) | nl-be |
| DE Germany | EUR | 19 / 7 / 0 | DATEV SKR 03 | 9930 (USt-IdNr) | de |
| DK Denmark | DKK | 25 (no reduced band) | Standardkontoplan-aligned | 0184 (CVR) | da |
| FI Finland | EUR | 25.5 / 13.5 / 10 / 0 | Liikekirjuri model chart | 0037 (LY-tunnus) | fi |
| NO Norway | NOK | 25 / 15 / 12 / 0 | NS 4102 standard kontoplan | 0192 (org.nr) | nb |
| SE Sweden | SEK | 25 / 12 / 6 / 0 | BAS 2023 | 0007 (org.nr) | sv |
| AT Austria | EUR | 20 / 13 / 10 | Einheitskontenrahmen (EKR) | 9914 (UID) | de |
| IE Ireland | EUR | 23 / 13.5 / 9 / 4.8 / 0 | UK-style (no statutory chart) | 9935 (VAT number) | en |
| IT Italy | EUR | 22 / 10 / 5 / 4 | Commercialisti convention (no statutory chart) | 0211 (Partita IVA) | it |
| ES Spain | EUR | 21 / 10 / 4 | PGC (R.D. 1514/2007) | 9920 (NIF) | es |
| PT Portugal | EUR | 23 / 13 / 6 | SNC (DL 158/2009) | 9946 (NIPC) | pt |
| BG Bulgaria | EUR | 20 / 9 | NSS (statutory, simplified) | 9926 (VAT) | en |
| HR Croatia | EUR | 25 / 13 / 5 | Računski plan (convention) | 9934 (VAT) | en |
| SI Slovenia | EUR | 22 / 9.5 / 5 | SRS 30 (standardised) | 9949 (VAT) | en |
| EE Estonia | EUR | 24 / 9 | RMP convention | 9931 (VAT) | en |
| LV Latvia | EUR | 21 / 12 / 5 | Standard kontu plāns | 9939 (VAT) | en |
| LT Lithuania | EUR | 21 / 9 / 5 | Įmonių sąskaitų planas (MF) | 9937 (VAT) | en |
| MT Malta | EUR | 18 / 12 / 7 / 5 | Convention (English) | 9943 (VAT) | en |
| CY Cyprus | EUR | 19 / 9 / 5 / 3 | Convention (English) | 9928 (VAT) | en |
| CZ Czechia | CZK | 21 / 12 | Směrná účtová osnova (statutory) | 9929 (VAT) | en |
| SK Slovakia | EUR | 23 / 19 / 5 | Směrná účtová osnova (statutory) | 9950 (VAT) | en |
| GR Greece | EUR | 24 / 13 / 6 | ΕΓΛΣ (statutory) | 9933 (VAT) | en |
| PL Poland | PLN | 23 / 8 / 5 | Rozporządzenie MF (statutory) | 9945 (VAT) | en |
| HU Hungary | HUF | 27 / 18 / 5 | Szt. (statutory) | 9910 (VAT) | en |
| RO Romania | RON | 18 / 9 | Planul de conturi (statutory) | 9947 (VAT) | en |

---

## Roadmap

| Phase | Scope | Status |
|-------|-------|--------|
| 0 | Foundation: ledger, posting engine, audit, trial balance, `--json`/`--dry-run` | ✅ done |
| 1 | Accounts CRUD + CSV import, RGS-mapped chart, balans + W&V, CSV/XLSX export, backup/restore | ✅ done |
| 2 | Bank import (CAMT.053/CSV), matching; optional VAT module (codes, OB readout, KOR) | ✅ done |
| 3 | Invoicing: factuurvereisten, PDF (Playwright), UBL/Peppol BIS 3.0, credit notes, payment matching, recurring entries **+ recurring invoices + Peppol send** | Compliant invoice PDF + UBL per invoice; due entries generated & posted on time | ✅ done (v0.6.0) |
| 4 | Jaarrekening micro/klein models, closing entries, KVK package, ICP readout | Jaarrekening package for a micro BV — **✅ done (v0.7.0, 178 tests green)** | planned |
| 5 | Agent layer: MCP server, permissions/approval gates, NL query, AI categorization suggestions, compliance calendar, FX translation | Agent closes a month end-to-end with zero unsupervised mutations — **✅ done (v0.8.0, 199 tests green)**; actor identity (signed commands, `audit verify`) + per-actor authorizations **✅ done (v0.15.0)** | planned |
| 6 | Migration & automation: `import opening-balances`, `import journal` (SnelStart/Exact CSV), `import xaf` (XML Auditfile 4.0), `month-end` close check, `invoice reminders` | Switch from an old package in one morning; the agent runs the close check monthly — **✅ done (v0.9.0, 229 tests green)** | planned |
| 7 | Fixed assets: depreciation schemes (lineair/degressief), asset register with mid-life adoption, monthly runs, disposal, activastaat | Recognise mid-life assets and book only the remaining depreciation — **✅ done (v0.10.0, 271 tests green)** | planned |
| 8 | SEPA payment batches: payables register (transfer vs direct-debit), pain.001 export for bank-portal upload | Prepare vendor payments in bukio, upload the file in the bank, close the loop via the CAMT import — **✅ done (v0.11.0, 295 tests green)** | planned |
| 9 | External handover: `export xaf` (Auditfile Financieel 4.0) + audit log as csv/xlsx | The year as a file your boekhouder/tax advisor/auditor imports directly — **✅ done (v0.12.0, 342 tests green)** | planned |
| 10 | Optional: Ponto live feeds, Peppol send/receive, OCR, SQLCipher | optional |
| 11 | Items catalog + discounts + invoice languages: `item` CRUD, `invoice create --items/--discount-*/--language`, fractional quantities, per-line + total discounts with per-rate VAT allocation, VAT breakdown per rate on PDF/UBL, company logo on the PDF | Invoice from a reusable catalog with discounts, in Dutch or English, with the company logo — **✅ done (v0.13.0, 433 tests green)** | planned |
| 12 | Inbound e-invoicing + delivery + cash management: attachments in-DB (`attach`), encrypted/rotated backups, aging/statement/sales reports, `import invoice` (EN 16931/Peppol UBL → payables), `invoice email` (SMTP), SEPA direct debit (`mandate` + pain.008) | The 2027 e-invoice mandate both ways: receive UBL invoices, email the PDF, collect by incasso — **✅ done (v0.14.1, 603 tests green)** | planned |
| 13 | Actor security layers: **Tier 0** signed actor commands (per-company key registry, enforcement, `audit verify`) + **Tier 0.5 per-actor authorizations** — capability families + roles (`actor authz`, `actor roles`, `actor can`, `actor who-can`), deny-by-default segregation-of-duties gate in the sign gate (CLI + MCP), owner-mediated key revoke | Every command signed and attributable; agents act only within their role — the actor who books is not the one who files or pays — **✅ done (dev branch, 746 tests green)** | done |
| 14 | Multi-jurisdiction profiles: sixteen (NL + the fifteen-market expansion LU/GB/FR/US/BE/DE/DK/FI/NO/SE/AT/IE/IT/ES/PT) — country chart conventions, VAT codes + rates, identifiers + Peppol schemes, compliance calendars; strict format dispatch (unbuilt formats fail loudly, no silent fallbacks) | One research-verified profile per market (docs-research/*.md); PLANNED empty (CH parked) — **✅ done (dev branch, 932 tests green)** | done |
| 15 | Localization (i18n): optional `--locale` / `BUKIO_LOCALE` mechanism with English default + locale tables covering all twenty-four markets' languages (en, nl, nl-be, de, fr, fr-lu, da, fi, nb, sv — AT resolves to de, IE to en); curated wiring of PDF labels, emails, CLI renders, VAT descriptions | English default, opt-in per market — **✅ done (dev branch, 932 tests green)** | done |
| 16 | Phase C: AT Austria (EKR chart, USt 20/10/13, Kleinunternehmer ≤ €55K, UID/FN, UVA, Peppol) + IE Ireland (UK-style chart, VAT 23/13.5/9/4.8/0, CRO + IE VAT, VAT3 bi-monthly, Peppol) — profiles, research briefs, contract tests | Thirteen markets live; research briefs at docs-research/{at,ie}-profile.md — **✅ done (dev branch, 932 tests green)** | done |
| 17 | Phase D: IT Italy (convention chart, IVA 22/10/5/4, Partita IVA, liquidazione IVA quarterly 16th + Dichiarazione 30 Apr, FatturaPA/SdI domestic e-invoicing as B-milestone) + ES Spain (PGC chart, IVA 21/10/4, NIF, Modelo 303/390, Verifactu B-milestone) + PT Portugal (SNC chart, IVA 23/13/6, NIPC, Declaração Periódica, ATCUD B-milestone) — profiles, research briefs, contract tests | Sixteen markets live; research briefs at docs-research/{it,es,pt}-profile.md — **✅ done (dev branch, 932 tests green)** | done |
| 18 | Phase E: BG Bulgaria + HR Croatia + SI Slovenia + EE Estonia + LV Latvia + LT Lithuania + MT Malta + CY Cyprus — eight more EUR-market profiles (EAS codes verified against the official OpenPEPPOL codelist; monthly/quarterly VAT deadlines; annual accounts + CIT per market) | Thirty markets live; research briefs at docs-research/{bg,hr,si,ee,lv,lt,mt,cy}-profile.md — **✅ done (dev branch, 932 tests green)** | done |
| 19 | Phase F: CZ Czechia + SK Slovakia + GR Greece + PL Poland + HU Hungary + RO Romania — the final six EU members (CZK/PLN/HUF/RON base currencies; GR uses the EL prefix; RO non-Peppol/e-Factura) | Thirty markets live — 27/27 EU + GB/NO/US; research briefs at docs-research/{cz,sk,gr,pl,hu,ro}-profile.md — **✅ done (dev branch, 944 tests green)** | done |

Design principles persist across phases: **agent-native from day one**, **VAT optional**, **no automated tax filing**, **single company per database**, **local-first**.

---

*Part of the Bukio product line — separate from the Bukio web platform: shared brand and philosophy, no shared code.*

---

**bukio-cli is provided completely open source and free** by [Posthuman Resources](https://agentic.bukio.nl). No license fees, no account, no cloud dependency — clone it, audit it, run it yourself.
