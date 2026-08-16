# Changelog

All notable changes to **bukio-cli** are recorded in this file. The format
loosely follows [Keep a Changelog](https://keepachangelog.com/); versions
match `package.json` and are bumped at release time. Work in progress on the
`dev` branch lives under **[Unreleased]** and moves to a version heading when
merged to `main` and released.

## [0.16.2] — 2026-08-16

### Added

- **Phase E: BG Bulgaria + HR Croatia + SI Slovenia + EE Estonia + LV Latvia
  + LT Lithuania + MT Malta + CY Cyprus** — eight more EUR-market profiles
  (twenty-four live): research briefs at
  `docs-research/{bg,hr,si,ee,lv,lt,mt,cy}-profile.md` (EAS codes verified
  against the official OpenPEPPOL codelist release 8 Dec 2025 — BG 9926,
  HR 9934, SI 9949, EE 9931, LV 9939, LT 9937, MT 9943, CY 9928);
  monthly VAT returns (BG 14th, HR/SI/EE/LV 20th, LT 25th; MT/CY
  quarterly 15th/10th of the second month), annual-accounts + CIT
  deadlines per market (BG 30 Jun, HR 30 Apr, SI 31 Aug/31 Mar, EE 30 Jun
  (CIT on distributions), LV 31 Jul (CIT on distributions), LT 30 Apr/1
  Oct, MT 10/9 months, CY 10 months/31 Jan+2y). New markets keep
  English-document defaults (no i18n tables yet — same treatment as
  GB/IE/US) and inherit the art. 226 EU baseline + cross-border Peppol
  BIS; domestic e-invoicing mandates (e-Sąskaita LT, PVN LV 2025
  framework) and return layouts stay documented B-milestones.

- **Phase F: CZ Czechia + SK Slovakia + GR Greece + PL Poland + HU Hungary
  + RO Romania** — the final six EU members (thirty live markets; 27/27 EU
  + GB/NO/US): research briefs at
  `docs-research/{cz,sk,gr,pl,hu,ro}-profile.md` (EAS codes verified
  against the official OpenPEPPOL codelist release 8 Dec 2025 — CZ 9929,
  SK 9950, GR 9933, PL 9945, HU 9910, RO 9947); monthly VAT returns
  (CZ/SK/PL/RO 25th, GR 26th, HU 20th), annual-accounts + CIT deadlines
  per market (CZ 6mo/31 Mar, SK 6mo/31 Mar, GR 10mo/30 Jun, PL 6mo/31
  Mar, HU 31 May/31 May, RO ~150d/25 Jun); CZK/PLN/HUF/RON base
  currencies via per-profile baseCurrency; GR uses the EL VIES prefix;
  RO is the only non-Peppol market (e-Factura national, cross-border UBL
  only); domestic mandates (KSeF PL — live 2026, RTIR HU, myDATA GR,
  e-Factura RO, SK 2027) stay documented B-milestones. English-document
  defaults; art. 226 EU baseline + cross-border Peppol BIS inherited.

- **i18n refactor: per-language table modules** — `src/i18n/index.js`'s
  single `TABLES` literal split into `src/i18n/locales/<lang>.js` (one
  module per language: en/nl/nl-be/de/fr/fr-lu/da/fi/nb/sv/it/es/pt);
  `index.js` keeps the machinery (`t`/`label`/`unitLabel`/`resolveLocale`,
  LABELS/UNITS backwards-compat exports) and re-assembles `TABLES` from
  imports. Public API unchanged; keys/values byte-identical; the parity
  guard still pins 89 keys across all 11 full tables. Behavior-neutral —
  944/944 green.

- **i18n: 14 national-language PDF tables** — full 89-key tables for
  pl/cs/hu/ro/sk/sl/hr/bg/el/et/lv/lt/mt/cy under `src/i18n/locales/`
  (25 full tables + 2 regional overrides; parity guard extended). The
  Phase E/F profiles now render documents in their national language by
  default (PL `pl`, CZ `cs`, SK `sk`, GR `el`, HU `hu`, RO `ro`, BG `bg`,
  HR `hr`, SI `sl`, EE `et`, LV `lv`, LT `lt`, MT `mt`, CY `cy`) — the
  invoice PDF/email pipeline is fully localized for all 30 markets;
  `--language` accepts any of the 27 table codes. `meta.locale` aligned
  to ISO language codes (si->sl, ee->et, cz->cs, gr->el).

## [0.16.1] — 2026-08-16

### Added

- **Phase C: AT Austria + IE Ireland** — two more jurisdiction profiles
  (thirteen live):
  - **AT** — Einheitskontenrahmen (EKR) chart (BMF SAF-T verified, 3-digit
    codes zero-padded), USt 20/13/10, Kleinunternehmer ≤ €55K, UID
    (ATU + 8 digits) / Peppol 9914, UVA quarterly (15th of the second
    following month; monthly above €100K) + annual USt-Erklärung 30 Jun.
  - **IE** — UK-style chart (no statutory chart), VAT 23/13.5/9/4.8/0,
    registration thresholds €85K goods / €42.5K services, CRO + IE-format
    VAT number / Peppol 9935, VAT3 bi-monthly returns (23rd of the month
    after the period; the `YYYY-Pn` shape) + annual accounts/CT1 in 9 months.
  - Research briefs at `docs-research/{at,ie}-profile.md`; strict dispatch
    keeps the B-milestones loud (UVA/VAT3 return engines, UGB/CA 2014
    accounts, SAF-T AT (OECD-style ≠ Auditfile), § 11 UStG / s. 108B VATCA
    invoice rule sets).
- **PLANNED now holds Phase D** (`IT`, `ES`, `PT`) — `init --country`
  answers `COUNTRY_NOT_SUPPORTED` for them (CH remains parked).
- **Profile-aware init output** — the `vat: module enabled` line shows the
  profile's clearing accounts (NL 1500/2500, AT 2500/3500, IE 2110/2100, …)
  instead of a hardcoded NL pair.
- **Clearer B-milestone errors** — `FORMAT_NOT_SUPPORTED` now names the
  country and the milestone ("no invoice compliance rule set for AT yet…",
  "no VAT-return layout for IE yet…") instead of `'undefined'`.
- **Phase D: IT Italy + ES Spain + PT Portugal** — three more jurisdiction
  profiles (sixteen live):
  - **IT** — commercialisti convention chart (no statutory chart; standard
    Italian account names), IVA 22/10/5/4, regime forfettario ≤ €85K,
    Partita IVA (IT + 11 digits) / Peppol 0211, liquidazione IVA quarterly
    (16th of the second month after the quarter) + Dichiarazione IVA 30 Apr
    + bilancio ~5 months.
  - **ES** — official PGC chart (R.D. 1514/2007), IVA 21/10/4, recargo de
    equivalencia, NIF / Peppol 9920, Modelo 303 quarterly (20th; Q4 30 Jan)
    + 390 (30 Jan) + 200 (25 Jul) + cuentas anuales (7 months).
  - **PT** — official SNC chart (DL 158/2009; 2-digit bases zero-padded),
    IVA 23/13/6, isenção art. 53 CIVA, NIPC / Peppol 9946, Declaração
    Periódica quarterly (20th of the second month) + IRC (31 May) + IES
    (15 Jul).
  - Research briefs at `docs-research/{it,es,pt}-profile.md`; strict
    dispatch keeps the B-milestones loud (liquidazione/303/DP return
    engines, bilancio/cuentas/demonstrações financeiras, SAF-T PT,
    **FatturaPA/SdI (IT)**, Verifactu (ES), ATCUD (PT) — domestic
    e-invoicing formats; Peppol BIS registered for cross-border).
- **Fully localised invoice PDFs** — the rendered invoice is now localised
  in every market language. New full i18n tables **it/es/pt** (88 keys each,
  key-identical to the pivot — parity guard now covers 11 full tables);
  the document language follows the company profile (`de-AT` → de, `it` →
  it, `nl-be` → nl, …) with `--language` accepting any i18n table;
  migration 025 rebuilds `invoices` to drop the stale `CHECK (language IN
  ('nl','en'))` that rejected `de`/`it` at INSERT.
- **EU invoice-compliance baseline** — the harmonized art. 226 VAT Directive
  party requirements ('eu-invoice-vereisten') are registered for the twelve
  EU markets without a national rule set (AT/BE/DE/DK/ES/FI/FR/IE/IT/NO/PT/
  SE), so invoice finalization — and thus the localised PDF — works for
  every EU market; NL/LU keep their national rule sets; GB/US stay
  B-milestone (no EU baseline).

## [0.16.0] — 2026-08-15

### Added

- **Eleven jurisdictions** — `bukio init --country <cc>` now seeds the country
  profile's chart convention, VAT codes/rates (2026), identifiers, Peppol
  scheme and compliance calendar for NL, LU, GB, FR, US, BE, DE, DK, FI, NO
  and SE (Phase B; profiles at `src/jurisdictions/`, roadmap rows 14–15):
  - NL: 29-account RGS-mapped chart (unchanged baseline, byte-identical),
    KvK/btw-id, Peppol 9944.
  - LU: PCN 2020 chart, RCS, Peppol 0195; LU e-invoicing rules.
  - GB: QuickBooks/Xero-style chart, Companies House number, Peppol 0208,
    MTD-compatible stance, 03-31 fiscal year.
  - FR: PCG chart, SIREN, Peppol 0002, franchise small-business scheme.
  - US: no-federal-VAT tracking model (track + export).
  - BE: PCN-BE AR 12-09-1983 chart, KBO, Peppol 0208, BTW readout.
  - DE/DK/FI/NO/SE: SKR 03 / standard charts, USt-IdNr. / CVR / Y-tunnus /
    Org.nr., Peppol 9930 / 0184 / 0037 / 0192 / 0007, per-market VAT bands
    (incl. DK 25 % only, NO no-reduced band, DE 0 % solar, FI 25.5 %).
  - Strict dispatch: unregistered formats fail loudly (`PROFILE_NOT_FOUND`),
    never a silent NL fallback — NL is one of eleven equal citizens.
- **Localization (i18n)** — optional, opt-in `--locale <code>` (global flag)
  or `BUKIO_LOCALE` env switches human-facing output; **English stays the
  default** and the JSON contract, error codes and MCP tool names never
  localize:
  - Locale tables for en, nl, nl-be, de, fr, fr-lu, da, fi, nb, sv
    (8 full tables × 88 keys, parity-guarded by tests; nl-be/fr-lu as
    regional overrides).
  - Wired: invoice PDF labels/units, invoice + reminder emails, CLI tables
    and renders (invoice list/reminders, balance sheet, P&L, month-end,
    year-end, company show), VAT file/settle descriptions. `--desc` still
    wins over localized defaults.
  - Invoice document language follows the company profile (Dutch for NL/BE
    companies, English for every other market; `--language nl|en` overrides)
    — no market is the de facto base.
- **English-first terminology sweep** — generic all-market runtime strings
  (labels, plans, help, MCP descriptions) are English by default; statutory
  artifacts keep their legal language per market (OB readout labels,
  'Winst en verlies' statutory XLSX sheet, RGS group names, command aliases,
  import CSV header aliases).
- **`financial-statements` command** (year-end close) with the deprecated
  `jaarrekening` alias; `init --kvk/--btw-id` deprecated in favour of
  `--registration-id/--tax-id`.

### Changed
- **Positioning** — repository, README and CLI description now state the
  eleven-jurisdiction scope (the "Dutch SMEs" tagline is gone); the invoice
  document-language default is documented as profile-derived (Dutch for NL/BE
  companies, English otherwise). The README presents the Netherlands as one of
  eleven equal citizens: `init`/`company`/`account`/`contact` options use the
  profile-neutral names (`--country`, `--registration-id`, `--tax-id`,
  `--taxonomy-code`), chart and report examples are marked as the NL profile's,
  and the OB readout is framed as the NL statutory VAT-return shape. The
  command reference was re-verified against the live CLI (init/company/
  account/contact option names, invoice create + bank import + fx set options,
  per-profile VAT codes and statutory models).

- Migrations 021–024: company gains `country`, `base_currency`, `locale`,
  `profile_version`; `legal_form` CHECK removed; `taxonomy` backfill for RGS;
  `vat_returns`/`filings` type CHECK widened — lossless, verified.
- Code comments remain Dutch where they explain statutory/fiscal mechanics;
  the public surface is English.

### Fixed

- `invoice show` no longer throws `ReferenceError` on invoices with payments
  or outstanding amounts in human mode (locale was undeclared in the render
  path; JSON mode had masked it).
- i18n table parity repair (the 14 balance-sheet/company keys now exist in
  all 8 full tables; a parity-guard test prevents silent regressions).
- Residual Dutch literals on generic paths (assets list/schemes, fx, item
  list, ICP readout, import/export summaries, MCP tool descriptions).

### Verified

- **891/891 tests green** (was 876 at Phase B start; +15 market/i18n
  coverage). Two consecutive clean full-review passes (rounds 8+9) plus the
  round-10 fresh pass and the i18n 3-review chain — all fail-closed,
  static-only subagent reviews with file:line verdicts.

## [0.15.1] — 2026-08-11

### Added

- **Remote access** (`bukio server` + global `--server <url>` / `BUKIO_SERVER`):
  serve ONE company DB over HTTP(S) and drive it from any device — a phone,
  a laptop, an agent on another VPS — while private signing keys stay on the
  devices that own them.
  - `bukio server start --listen <host:port> [--serve-db <path>]
    [--tls-cert C --tls-key K]` — the daemon. Clients POST signed command
    envelopes to `/rpc`; the server verifies each against the company
    registry (Ed25519 signature over the canonical digest — same scheme as
    local signed commands — plus ±5 min timestamp window, nonce replay
    refusal and the Tier 0.5 authz gate) and runs it as a child CLI process.
    Output is byte-identical to local mode; audit rows carry the REAL
    signature, so `audit verify` validates remote commands exactly like local
    ones. Like `mcp`, the daemon is a bridge and needs no signature.
  - `bukio server token <actor> [--ttl-hours N]` — mint one-time enrolment
    tokens (single-use, TTL, actor-bound, stored sha256-hashed). Operator
    act on the server machine; redeem with `bukio actor register --server
    <url> --token <t>`. The token replaces the local
    enforce-off/register/enforce-on dance for remote first enrolment.
  - Global `--server <url>` / `BUKIO_SERVER` turns ANY command into a signed
    envelope: sign locally, verify + execute on the server, print the same
    output. Local-only commands (`server *`, `mcp`, `init`, `update`, `actor
    keygen/unlock/lock`) refuse with `LOCAL_ONLY`.
  - Same-device operation: no `--server` is the unchanged local path;
    `--server http://127.0.0.1:PORT` drives a local server identically.
  - Transport: plain HTTP for trusted networks (localhost, Tailscale) or
    native TLS via `--tls-cert/--tls-key`; envelope signatures provide
    authentication and integrity. New error codes `LOCAL_ONLY`,
    `REMOTE_UNREACHABLE`, `REMOTE_ERROR`, `TOKEN_REQUIRED/INVALID/EXPIRED/
    USED/ACTOR_MISMATCH`, `BAD_JSON`, `BODY_TOO_LARGE`,
    `INVALID_ENVELOPE`, `CMD_MISMATCH`, `INVALID_LISTEN`, `TLS_KEY_REQUIRED`,
    `SERVER_EXEC` — all documented in AGENTS.md §7.
  - New module `src/cli/server.js` (daemon + token minting), `src/cli/remote.js`
    (client envelope build/sign/POST) — `node:http`/`node:https`/
    `node:crypto` only, no new dependencies. 20 new e2e tests in
    `test/remote.test.js` (enrolment, replay, tamper, enforcement, authz,
    dry-run parity, human-output parity, same-device parity, LOCAL_ONLY,
    health).
  - README screenshot regenerated (v0.15.1) showing the remote-access flow
    end-to-end: `server start` → `server token` → `actor register --server`
    → `--server entry add` → remote trial balance → `audit verify`.

## [0.15.0] — 2026-08-11

Resolves **[#1 — Audit trail actor attribution is self-asserted](https://github.com/erikvankempen/bukio-cli/issues/1)**:
the recorded actor is now cryptographically verifiable (signed commands,
`audit verify`, replay refusal), unverifiable actions are marked
`unsigned` or refused under enforcement, and the mechanism works across
CLI + MCP for all three roles — with per-actor authorizations
(segregation of duties) layered on top.

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

### Fixed

- Aging report: the creditors leg now honours `--as-of` (payables dated after
  the as-of date were shown in historical positions — the debtors leg already
  filtered, the creditors leg did not).
- Invoice finalize: the sequential number is allocated **inside** the booking
  transaction with a UNIQUE-collision retry — two concurrent finalizes (CLI +
  MCP, or two processes on a WAL database) could allocate the same number and
  surface a raw SQLite error instead of a clean retry.
- `markPaid`: the payment insert and the status transition now commit in one
  transaction — a crash between them left a recorded payment with the invoice
  still `sent`.
- Recurring `createTemplate`: object postings mixed into a VAT-tagged posting
  list are kept instead of silently dropped by the expansion filter.
- MCP server: `tools/call` with `params: null` (sent by some JSON-RPC clients)
  answered with a -32603 internal error; it now returns a clean -32602
  invalid-params reply.
- `audit verify`: a negative `--limit` used to slice from the wrong end
  (silently dropping the oldest rows); it is now rejected with
  `INVALID_LIMIT`, matching `audit list`.

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
