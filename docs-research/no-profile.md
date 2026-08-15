# Norway (NO) — SME Bookkeeping Profile Research Brief

**Researched:** 2026-08-15 · **Purpose:** input for `bukio-cli` jurisdiction profile `no.js` (pattern: `lu.js`, `gb.js`, `fr.js`)
**Confidence legend:** HIGH = verified on official/primary source; MED = verified on 2+ credible secondary sources; LOW = single secondary source or convention knowledge, needs code-level verification.
**Flagged items:** any claim marked ⚠ could NOT be fully verified against a primary source within budget.

---

## 1. VAT (merverdiavgift / MVA) — rates, threshold, EU status

| Item | Value | Confidence | Source |
|---|---|---|---|
| Standard rate | **25 %** ("Normal rate") | HIGH | https://www.skatteetaten.no/en/rates/value-added-tax/ (rates page, year selector 2026) |
| Reduced rate — foodstuffs | **15 %** ("Foodstuffs") | HIGH | same |
| Reduced rate — water/wastewater | **15 %** (reduced from 25% on 1 July 2025) | HIGH | https://www.skatteetaten.no/en/rates/vat-rates-for-water-and-wastewater-services/ |
| Reduced rate — passenger transport, letting of rooms (accommodation), cinema, public broadcasting, entry to sporting events/amusement parks | **12 %** | HIGH | skatteetaten rates page (footnote) |
| 0% | Exempt sales ("avgiftsfri" / nullsats) and sales outside VAT scope exist (e.g. financial services, health) — no general 0% band | MED | https://www.sporo.no/kontoplan (3100 "Salgsinntekt, avgiftsfri", 3200 "Salgsinntekt, utenfor avgiftsområdet") |
| Registration threshold | **NOK 50,000** (excl. VAT) taxable turnover over any 12-month period; **NOK 140,000** for charitable/non-profit orgs | HIGH | https://www.skatteetaten.no/en/business-and-organisation/vat-and-duties/vat/register-change-delete/ ; https://taxsummaries.pwc.com/norway/corporate/other-taxes |
| EU membership | **Norway is NOT in the EU** (EEA member). **No EU reverse charge** mechanism; domestic reverse charge exists only for specific cases (e.g. remotely delivered services from abroad). | HIGH | context + https://info.altinn.no/en/start-and-run-business/direct-and-indirect-taxes/indirect-taxes/reporting-and-paying-vat/ ("Remote delivered services" section — purchases from abroad reported via VAT return, settlement at NOK 2,000/quarter) |
| VOEC (low-value imports) | **VOEC scheme** (Value added tax on e-commerce): foreign sellers of goods ≤ **NOK 3,000** (and remote services of any value) to Norwegian consumers do simplified VAT registration; VAT collected at sale, no import VAT at customs. Excludes food, tobacco, alcohol. | HIGH | Altinn page above; https://www.toll.no/en/corporate/import/the-voec-scheme ; https://www.skatteetaten.no/en/business-and-organisation/vat-and-duties/vat/foreign/e-commerce-voec/ |
| Small-business exemption scheme | **There is NO small-business VAT exemption scheme** (no counterpart to e.g. UK flat-rate/annual exemption). The NOK 50,000 limit is only a **registration obligation threshold** — once registered, VAT applies from the first krone; businesses below the threshold simply do not register. The only "simplification" is annual reporting (see §2). | HIGH | https://marosavat.com/vat-manual-chapters/norway-vat-registrations ("Small businesses … are not required to register …", no exemption); skatteetaten register page (threshold framed as registration duty, not exemption) |

**Note for profile:** `vatScheme` = `mva`; rates `{standard: 25, reduced_food: 15, reduced_other: 12}`; flag `eu: false`, `reverseCharge: false`, `voec: true` (import side), `smallBusinessExemption: false`.

---

## 2. VAT return — terminoppgave / mva-meldingen

- The VAT return is called **terminoppgave** / now **"mva-meldingen"** (Tax return for VAT), filed via **Altinn** (or directly from accounting software/SAF-T).
- **⚠ PREMISE CORRECTION:** filing is **NOT monthly**. The standard frequency is **bi-monthly (every other month) — 6 returns per year**. Monthly filing is not the Norwegian norm (the "monthly since 2022" rule the task assumed applies to EU countries like Austria, not Norway). | HIGH | https://info.altinn.no/en/start-and-run-business/direct-and-indirect-taxes/indirect-taxes/reporting-and-paying-vat/ ("must submit tax returns for VAT … six times a year")
- **Deadline: within one month and ten days after the end of each period** — i.e. normally the 10th of the second month after the period. Fixed schedule:
  - Period 1 (Jan/Feb) → **10 April**
  - Period 2 (Mar/Apr) → **10 June**
  - Period 3 (May/Jun) → **31 August** (exception to the 10th rule — summer deadline)
  - Period 4 (Jul/Aug) → **10 October**
  - Period 5 (Sep/Oct) → **10 December**
  - Period 6 (Nov/Dec) → **10 February**
  | HIGH | Altinn page above (deadlines list); https://www.eurofiscalis.com/en/norway-vat-return/ (31 Aug exception)
- Return must be filed even with zero sales. | HIGH | Altinn page above
- **Annual reporting option:** after ≥12 months registered, enterprises with turnover < **NOK 1 million** may apply to report annually; annual return + payment due **10 March** of the following year; application window 10 Dec–1 Feb. | HIGH | Altinn page; https://www.skatteetaten.no/en/business-and-organisation/vat-and-duties/vat/paying-vat/small-enterprises-can-apply-to-report-vat-once-a-year/
- Primary industries (agriculture/forestry/fishing): annual return due 10 April. | HIGH | Altinn page
- **So for the profile:** `filingFrequency: "bimonthly"`, `dueRule: "1 month + 10 days after period end"`, `annualOptionBelow: 1000000`. (Example given in the task — "January period due 10 March" — only matches the **annual** return, not a monthly cycle.)

---

## 3. Identifiers

| Identifier | Value | Confidence | Source |
|---|---|---|---|
| Company number | **Organisasjonsnummer (org.nr)** — 9 digits, last digit a Mod-11 check digit, displayed as `123 456 789` | HIGH | https://www.brreg.no/en/about-us-2/our-registers/about-the-central-coordinating-register-for-legal-entities-ccr/about-the-organisation-number/ |
| VAT number | **`NO` + 9-digit org.nr + `MVA`** → `NO123456789MVA`; domestic display `123 456 789 MVA`. The NO prefix is used on international/EU-facing documents; the MVA suffix identifies the VAT registration (same number as org.nr — not a separate ID). | HIGH | https://peppolvalidator.com/peppol-validation-errors/NO-R-001 (Peppol rule: MUST be NO+9digits+MVA); https://www.commenda.io/blog/norway-vat-number-verification ; https://www.eurofiscalis.com/en/invoice-requirements-norway/ |
| Peppol scheme ID | **0192** — identifier scheme `NO:ORG` (Organisasjonsnummer). Participant ID format `0192:<9-digit org.nr>` | HIGH | https://docs.peppol.eu/edelivery/codelists/old/v8.5/Peppol%20Code%20Lists%20-%20Participant%20identifier%20schemes%20v8.5.html (0192 → NO:ORG); https://peppolgate.eu/peppol-country-codes/ |
| Bank account | Domestic: **11-digit account number (kontonummer)** = 4-digit bank group code + 6 digits + 1 check digit. SEPA/IBAN: **`NO` + 2 check digits + 11-digit BBAN = 15 chars** (e.g. `NO93 8601 1117 947`). | MED (structure); IBAN 15-char HIGH | https://wise.com/us/iban/norway (15 chars, NO); https://www.xflowpay.com/tools/iban/countries/norway/bank-norwegian ("11-digit BBAN (kontonummer)"); ⚠ 4-digit bank-group-code + 6-digit split is standard knowledge — not explicitly verified in a primary source within budget |

`accountNumber.kind` for the profile: **iban** (bank-level; domestic account object should carry the 11-digit `kontonummer`).

---

## 4. Legal forms (organisasjonsformer)

| Code | Norwegian | English | Notes | Confidence | Source |
|---|---|---|---|---|---|
| **AS** | Aksjeselskap | Limited company | **The SME standard**; min share capital NOK 30,000 (2012+); separate legal entity, limited liability | HIGH | https://www.brreg.no/en/business-2/types-of-organisation/ |
| **ENK** | Enkeltpersonforetak | Sole proprietorship | Owner fully liable; used for the profile's personal/SME tier | HIGH | brreg types page |
| **ANS** | Ansvarlig selskap | General partnership | Unlimited joint liability of partners | HIGH | brreg types page; https://info.altinn.no/starte-og-drive/starte/valg-av-organisasjonsform/ansvarlig-selskap/ |
| **DA** | Selskap med delt ansvar | General partnership with divided liability | Partners liable only for own share | HIGH | brreg types page |
| **NUF** | Norskregistrert utenlandsk foretak | Norwegian-registered foreign company (branch) | Foreign entity with Norwegian registration; needs a Norwegian contact person (for VAT: a representative unless EEA/UK resident) | HIGH | brreg types page; skatteetaten register page ("Registration for foreign enterprises") |

---

## 5. Chart of accounts — Norsk Standard Kontoplan (NS 4102)

- Norway has **no statutory chart of accounts**; the de-facto convention is **NS 4102 "Norsk Standard Kontoplan"**, used (with minor per-vendor numbering shifts) by **Visma eAccounting, Tripletex, Fiken, Sporo** and most SME packages. Class prefix: `1xxx` assets, `2xxx` liabilities & equity, `3xxx` revenue, `4xxx` COGS, `5xxx` payroll, `6xxx` operating costs, `7xxx` other operating costs, `8xxx` financial items & tax. | HIGH | https://www.sporo.no/kontoplan ("Norsk standard kontoplan (NS 4102)"); Tripletex Kontohjelp (https://www.tripletex.no/kontohjelp/konto/2700-utgaende-mva-hoy-sats/); Fiken Kontohjelp (https://kontohjelp.fiken.no/)
- **⚠ Vendor variance in MVA account numbering:** Tripletex & NS 4102 (sporo) use `2700`/`2710`; **Fiken uses `2701` Utgående mva høy sats / `2711` Inngående mva høy sats**. Recommend the NS 4102 numbering (`2700/2710/2740`) as the profile default. | MED | sporo kontoplan page; https://kontohjelp.fiken.no/as/medMva/2701 (search result title/desc); Tripletex 2700 page

### Default chart (40 accounts, NS 4102 codes — verified list from https://www.sporo.no/kontoplan)

Type legend: A=asset, L=liability, E=equity, R=revenue, X=expense, C=closing. Normal balance: D=debit, C=credit.

| Code | Norwegian label | English | Type | Bal | MVA (per NS 4102) |
|---|---|---|---|---|---|
| 1200 | Maskiner og anlegg | Machinery & plant | A | D | — |
| 1220 | Inventar og utstyr | Furniture & equipment | A | D | — |
| 1250 | IT-utstyr | IT equipment | A | D | — |
| 1300 | Investeringer i aksjer | Investments in shares | A | D | — |
| 1500 | Kundefordringer | Accounts receivable | A | D | — |
| 1570 | Andre kortsiktige fordringer | Other short-term receivables | A | D | — |
| 1700 | Forskuddsbetalt leie | Prepaid rent | A | D | — |
| 1900 | Kontanter | Cash on hand | A | D | — |
| 1920 | Bankinnskudd | Bank deposits (main) | A | D | — |
| 1921 | Bankinnskudd 2 | Bank deposits (2nd account) | A | D | — |
| 1925 | Skattetrekkskonto | Tax-withholding account (payroll) | A | D | — |
| 2000 | Aksjekapital | Share capital (AS) | E | C | — |
| 2020 | Overkursfond | Share premium fund | E | C | — |
| 2050 | Egenkapital | Owner's equity (ENK) | E | C | — |
| 2060 | Privatuttak | Owner drawings (ENK) | E | D | — |
| 2080 | Privat innskudd | Owner deposits (ENK) | E | C | — |
| 2400 | Leverandørgjeld | Accounts payable | L | C | — |
| 2500 | Betalbar skatt | Income tax payable | L | C | — |
| 2600 | Skattetrekk | Payroll tax withheld (employee) | L | C | — |
| 2700 | Utgående MVA, høy sats | Output VAT 25% | L | C | (output) |
| 2701 | Utgående MVA, middels sats | Output VAT 12% | L | C | (output) |
| 2702 | Utgående MVA, lav sats | Output VAT 15% | L | C | (output) |
| 2704 | Utgående MVA, tjenester utlandet | Output VAT, foreign services | L | C | (output) |
| 2710 | Inngående MVA, høy sats | Input VAT 25% | A | D | (input) |
| 2711 | Inngående MVA, middels sats | Input VAT 12% | A | D | (input) |
| 2712 | Inngående MVA, lav sats | Input VAT 15% | A | D | (input) |
| 2740 | Oppgjørskonto MVA | VAT settlement account | L | C | — |
| 2800 | Utbytte | Dividends declared | E | D | — |
| 2910 | Forskudd fra kunder | Advances from customers | L | C | — |
| 2960 | Påløpte kostnader | Accrued expenses | L | C | — |
| 2990 | Annen kortsiktig gjeld | Other short-term debt | L | C | — |
| 3000 | Salgsinntekt, avgiftspliktig | Sales revenue, taxable | R | C | Output 25% |
| 3100 | Salgsinntekt, avgiftsfri | Sales revenue, exempt (0%) | R | C | Exempt |
| 3200 | Salgsinntekt, utenfor avgiftsområdet | Sales outside VAT scope | R | C | Outside |
| 3600 | Leieinntekt | Rental income | R | C | — |
| 3900 | Annen driftsinntekt | Other operating income | R | C | — |
| 4000 | Varekjøp | Purchases of goods (COGS) | X | D | Input 25% |
| 4500 | Fremmedytelser og underentreprise | Subcontracted services | X | D | Input 25% |
| 5000 | Lønn | Wages | X | D | — |
| 5090 | Feriepenger | Holiday pay | X | D | — |
| 5400 | Arbeidsgiveravgift | Employer's national insurance | X | D | — |
| 5800 | Pensjonskostnader | Pension costs | X | D | — |
| 5900 | Andre personalkostnader | Other personnel costs | X | D | — |
| 6000 | Avskrivning | Depreciation | X | D | — |
| 6100 | Frakt og transport | Freight & transport | X | D | Input 25% |
| 6200 | Elektrisitet | Electricity | X | D | Input 25% |
| 6300 | Leie lokale | Rent of premises | X | D | — |
| 6400 | Leie maskiner, inventar | Equipment rental | X | D | Input 25% |
| 6500 | Inventar og utstyr (kostnadsføres) | Minor equipment expensed | X | D | Input 25% |
| 6550 | Programvare | Software | X | D | Input 25% |
| 6600 | Reparasjon og vedlikehold | Repairs & maintenance | X | D | Input 25% |
| 6700 | Revisjonshonorar | Audit fees | X | D | Input 25% |
| 6720 | Regnskapshonorar | Accounting fees | X | D | Input 25% |
| 6790 | Andre fremmedtjenester | Other external services | X | D | Input 25% |
| 6800 | Kontorrekvisita | Office supplies | X | D | Input 25% |
| 6900 | Telefon | Telephone | X | D | Input 25% |
| 7100 | Bilgodtgjørelse | Mileage allowance | X | D | — |
| 7140 | Reisekostnad | Travel costs | X | D | — |
| 7300 | Markedsføring | Marketing | X | D | Input 25% |
| 7350 | Representasjon | Representation | X | D | — |
| 7500 | Forsikring | Insurance | X | D | — |
| 7770 | Bank- og kortgebyrer | Bank & card fees | X | D | — |
| 7790 | Andre driftskostnader | Other operating costs | X | D | — |
| 8040 | Renteinntekt | Interest income | R | C | — |
| 8140 | Rentekostnad | Interest expense | X | D | — |
| 8160 | Valutatap (disagio) | FX loss | X | D | — |
| 8300 | Betalbar skatt | Current tax expense | X | D | — |
| 8960 | Overført til egenkapital | Transferred to equity (closing) | C | C | — |

For the SME default chart pick ~40: all of 2xxx VAT/equity core, 1920/1921/1500/2400/2600, revenue 3000/3100/3200/3900, COGS 4000/4500, payroll 5000/5090/5400, operating 6000/6100/6300/6400/6550/6600/6720/6790/6800/6900, other 7140/7300/7500/7770/7790, financial 8040/8140/8160, closing 8960.

---

## 6. VAT control accounts (Norwegian convention)

- **Output VAT = Utgående MVA** (balance, credit): `2700` høy sats (25%), `2701` middels sats (12%), `2702` lav sats (15%), `2704` foreign services, `2705` import of goods. | HIGH | sporo kontoplan; Tripletex Kontohjelp 2700
- **Input VAT = Inngående MVA** (balance, debit): `2710` høy sats, `2711` middels, `2712` lav, `2714` foreign services, `2715` import. | HIGH | sporo kontoplan; https://www.sporo.no/kontoplan/2710
- **Settlement = `2740 Oppgjørskonto MVA`** — clearing account that nets output vs input VAT per period and is settled against the tax authority (and mapped to the mva-meldingen). **Not 2710.** | HIGH | sporo kontoplan (2740 Oppgjørskonto MVA listed)
- **⚠ Fiken variant:** Fiken numbers output `2701`/input `2711` for high rate (https://kontohjelp.fiken.no/as/medMva/2701). Profile default = NS 4102 (`2700`/`2710`/`2740`); expose as config.
- VAT is handled via **MVA codes** (mva-koder) per rate in all three packages; the control accounts are locked to those codes in Tripletex ("Mva-kontoer i Tripletex er låst for føringer"). | MED | Tripletex 2700 page

---

## 7. Statutory accounts (årsregnskap / annual accounts)

- Regime: **årsregnskap** prepared under the Accounting Act (regnskapsloven); filed in the **Register of Company Accounts (Regnskapsregisteret)** at **Brønnøysund**. | HIGH | https://www.brreg.no/en/about-us-2/our-registers/about-the-register-of-company-accounts/
- **Approval:** annual accounts must be approved by the general meeting **no later than 6 months after the end of the financial year** (i.e. by 30 June for calendar year). | HIGH | https://info.altinn.no/en/start-and-run-business/accounts-and-auditing/accounting/annual-accounts/ ; https://www.skatteetaten.no/en/business-and-organisation/start-and-run/best-practices-accounting-and-cash-register-systems/annual-accounts/
- **Filing:** submitted **no later than 1 month after approval**; for calendar-year companies the effective register deadline is **31 July** (late-filing penalty thereafter). | HIGH | skatteetaten annual-accounts page; brreg register page ("due date for submission … 31 July")
- Audit: SMEs may be exempt from mandatory audit (turnover < NOK 5–6m / fewer than 3 employees depending on thresholds) — **⚠ exact 2026 thresholds not verified within budget**. | LOW | general knowledge — flag
- This is a **B-milestone** (layout/regime note only) — the profile need not build the annual report layout, just expose the regime + deadlines.

---

## 8. Fiscal year end

- **Default = calendar year, 1 January – 31 December** for the vast majority of companies. Non-calendar fiscal years require justification (e.g. parent abroad on a non-calendar year) and approval. | HIGH | https://taxsummaries.pwc.com/norway/corporate/tax-administration ("income tax year normally runs from 1 January to 31 December"); https://www.lloydsbanktrade.com/en/market-potential/norway/accounting ; https://www.brreg.no/en/submission-of-annual-accounts/reporting-obligations-to-the-register-of-company-accounts/non-calendar-fiscal-year/ (non-calendar year is the exception)
- Profile default: `fiscalYearEnd: "12-31"`.

---

## 9. Banking

- **SEPA member** (EEA): IBAN = `NO` + 2 check digits + 11-digit BBAN (15 chars total). Domestic transfers use the **11-digit kontonummer** (4-digit bank group code + 6-digit account + 1 check digit). | MED/HIGH | https://wise.com/us/iban/norway ; https://www.xflowpay.com/tools/iban/countries/norway/bank-norwegian ; ⚠ 4+6+1 split not confirmed on a primary source
- **CAMT.053** (ISO 20022 bank statement) is the standard statement format used by Norwegian banks and supported by Norwegian accounting software (Visma/Tripletex import bank statements in CAMT.053 / Bs2). | MED | ⚠ not explicitly verified for Norway within budget; CAMT.053 is the ISO standard — confirm at implementation time
- Norway additionally mandates **SAF-T Accounting** export for bookkeeping systems (relevant if the profile later adds e-reporting; out of scope for a B-milestone). | MED | https://edicomgroup.com/electronic-invoicing/norway (mentions SAF-T alongside MVA-melding) — flagged as side note

---

## 10. Compliance calendar (SME, calendar year)

| Date | Obligation | Confidence | Source |
|---|---|---|---|
| 10 Feb / 10 Apr / 10 Jun / **31 Aug** / 10 Oct / 10 Dec | VAT return + payment, bi-monthly periods (1m+10d after period end; May/Jun exception 31 Aug) | HIGH | Altinn VAT page |
| 10 Mar (following year) | Annual VAT return (small enterprises < NOK 1m, if opted in) | HIGH | Altinn page; skatteetaten |
| 10 Apr | Annual VAT return — primary industries | HIGH | Altinn page |
| 31 Jan | Shareholders' register return (aksjonærregisteroppgaven) | MED | https://efremtid.no/en/articles/company-startup-obligations-norway (secondary) |
| 30 Jun | Annual accounts approved by general meeting (≤ 6 months after FYE) | HIGH | Altinn annual-accounts page |
| 31 Jul | Annual accounts filed at Brønnøysund (≤ 1 month after approval; effective calendar-year deadline) | HIGH | brreg; skatteetaten |
| **31 May** (AS) / **30 Apr–31 May** (ENK) | Corporate income tax return (næringsoppgave / tax return) | LOW ⚠ | not verified within budget — verify at implementation |

---

## 11. e-Invoicing (EHF / Peppol)

- **B2G (public sector): mandatory since 2012** — all suppliers to Norwegian public entities must send structured electronic invoices via the Peppol network in **EHF** (Elektronisk Handelsformat; current EHF 3.0 = Peppol BIS Billing 3.0 UBL profile). | HIGH | https://www.e-invoice.app/guides/norway-e-invoicing ; https://peppolvalidator.com/peppol-norway ("mandatory for B2G in 2012"); https://www.theinvoicinghub.com/einvoicing-compliance-norway/
- **B2B: voluntary** — Peppol BIS 3.0 (UBL) and EHF 3.0 are the accepted formats; no B2B mandate yet. | HIGH | theinvoicinghub ("B2B … voluntary and widespread, very often using the local EHF 3.0 format"); e-invoice.app
- **⚠ Upcoming:** a B2B mandate is proposed for **2027–2030** (issuance→full scope) — announced, not yet law; flag for future-proofing. | MED | theinvoicinghub
- Peppol participant scheme: **0192 / NO:ORG** (see §3). VAT number on Peppol invoices must be `NO123456789MVA` (Peppol rule NO-R-001). | HIGH | https://peppolvalidator.com/peppol-validation-errors/NO-R-001

---

## 12. Closing accounts (year-end convention)

- **Result/transfer account:** NS 4102 uses **`8960 Overført til egenkapital`** ("Transferred to equity") as the closing entry that carries net result to equity. | HIGH | sporo kontoplan (8960 listed under Klasse 8)
- **⚠ Alternative conventions:** many packages expose **`8990 Årets resultat` / `8999`** as the annual-result account for the årsregnskap presentation, then transfer to equity. Not verified against a primary source within budget — make the result account configurable with `8960` as default.
- Equity targets: **`2000 Aksjekapital`** (AS) / **`2050 Egenkapital`** + `2060 Privatuttak`/`2080 Privat innskudd` (ENK). Retained earnings commonly under `2090 Annen egenkapital` (⚠ 2090 not present in sporo's NS 4102 list — vendor convention; keep configurable).
- Year-end sequence: P&L accounts → result account → transfer to equity (8960) → dividends (2800) if declared.

---

## 13. Currency / locale / formats

| Item | Value | Confidence |
|---|---|---|
| Currency | **NOK** (krone), ISO 4217 `NOK`, minor unit 100 øre, decimals 2 | HIGH (standard) |
| Locale | **nb-NO** (Bokmål) | HIGH (standard) |
| Date format | **dd.mm.yyyy** (e.g. 15.08.2026) | HIGH (Norwegian convention) |
| Decimal/group separators | `1 234 567,89` (space thousands, comma decimals) — matches org.nr display `123 456 789` | MED |

---

## Summary of unverified / flagged items

1. **Monthly VAT filing premise is wrong** — Norway is bi-monthly (6/yr); monthly applies to EU countries, not Norway. Annual option < NOK 1m.
2. **Fiken vs NS 4102 MVA account numbering** (2701/2711 vs 2700/2710) — vendor variance; NS 4102 chosen as default.
3. **`8990 Årets resultat` / `2090 Annen egenkapital`** — common but not in the verified NS 4102 source list; make configurable.
4. **11-digit kontonummer internal split (4+6+1)** and **CAMT.053 support specifics** — not verified on a primary source.
5. **Audit exemption thresholds** and **corporate tax return deadlines** (31 May etc.) — not verified within budget.
6. **B2B e-invoicing mandate 2027–2030** — announced/proposed, not law.
