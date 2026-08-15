# United Kingdom (England & Wales / Great Britain) SME Bookkeeping Profile — Research Brief for bukio-cli `gb.js` jurisdiction profile

**Research date:** 2026-08-15 · **Prepared for:** bukio-cli jurisdiction-profile layer (reference: `src/jurisdictions/lu.js`)
**Scope:** VAT rates/schemes/thresholds, identifiers, legal forms, statutory accounts regimes (FRS 102 §1A / FRS 105), e-invoicing & Making Tax Digital, default chart of accounts (no statutory chart — software convention), fiscal year-end conventions, banking rails, currency/locale. Every code/format below was verified against GOV.UK/HMRC/ICAEW/FRC or a reputable secondary source; confidence is marked per item.

## 0. Legal basis & sources

| # | Source | URL | Confidence |
|---|--------|-----|-----------|
| S1 | **GOV.UK** — VAT rates (20% standard / 5% reduced / 0% zero / exempt) | `https://www.gov.uk/vat-rates` | **High** (official) |
| S2 | **GOV.UK** — How VAT works: VAT thresholds (registration £90,000 / deregistration £88,000; scheme thresholds) | `https://www.gov.uk/how-vat-works/vat-thresholds` | High (official) |
| S3 | **GOV.UK** — Increasing the VAT registration and deregistration thresholds (£85K→£90K, £83K→£88K, from 1 April 2024) | `https://www.gov.uk/government/publications/vat-increasing-the-registration-and-deregistration-thresholds` | High (official) |
| S4 | **GOV.UK** — VAT Flat Rate Scheme (join ≤£150K; limited cost trader 16.5%) | `https://www.gov.uk/vat-flat-rate-scheme` + `https://www.gov.uk/vat-flat-rate-scheme/how-much-you-pay` | High (official) |
| S5 | **GOV.UK** — Sending a VAT Return (quarterly; deadline 1 month + 7 days) | `https://www.gov.uk/vat-returns` | High (official) |
| S6 | **GOV.UK** — VAT registration numbers: XI prefix for NI (example `XI 123456789` vs `GB 123456789`; identical digits, prefix differs) | `https://www.gov.uk/register-for-vat/selling-or-moving-goods-between-northern-ireland-and-the-eu` | High (official) |
| S7 | **GOV.UK** — Find your UTR number (10-digit Unique Taxpayer Reference; issued on SA registration or company setup) | `https://www.gov.uk/find-utr-number` | High (official) |
| S8 | **GOV.UK** — Set up a business (legal structures: sole trader, limited company, partnership, social enterprise) | `https://www.gov.uk/business-legal-structures` | High (official) |
| S9 | **GOV.UK** — Accounts and tax returns for private limited companies (9-month CH deadline; CT return 12 months; CT payment 9 months + 1 day) | `https://www.gov.uk/prepare-file-annual-accounts-for-limited-company` | High (official) |
| S10 | **GOV.UK** — Prepare annual accounts: micro-entities, small and dormant companies (size criteria; abridged filing options; balance sheet + notes for micro) | `https://www.gov.uk/annual-accounts/microentities-small-and-dormant-companies` | High (official) |
| S11 | **FRC** — FRS 105 (micro-entities regime standard) | `https://www.frc.org.uk/library/standards-codes-policy/accounting-and-reporting/uk-accounting-standards/frs-105/` | High (official standard-setter) |
| S12 | **ICAEW** — Small company filing options (FRS 102 §1A vs full FRS 102; abridged scenarios; balance-sheet statements) | `https://www.icaew.com/technical/corporate-reporting/uk-gaap/uk-gaap-faqs/small-company-filing-options` | High (reputable body) |
| S13 | **GOV.UK** — E-invoicing consultation response (mandatory e-invoicing for all VAT invoices **from 2029**, announced at Budget 2025; roadmap at Budget 2026) | `https://www.gov.uk/government/consultations/promoting-electronic-invoicing-across-uk-businesses-and-the-public-sector/outcome/promoting-electronic-invoicing-across-uk-businesses-and-the-public-sector-consultation-response` | High (official) |
| S14 | **GOV.UK** — Tax Update 2026 summary ("Peppol will be the core interoperability network for e-invoicing in the UK") | `https://www.gov.uk/government/publications/summary-of-tax-update-2026-simplification-modernisation-and-fairness/tax-update-2026-simplification-modernisation-and-fairness-summary` | High (official) |
| S15 | **GOV.UK** — MTD for Income Tax (from 6 Apr 2026 >£50K; 6 Apr 2027 >£30K; 6 Apr 2028 >£20K qualifying income) | `https://www.gov.uk/guidance/find-out-if-and-when-you-need-to-use-making-tax-digital-for-income-tax` | High (official, updated 26 Mar 2026) |
| S16 | **GOV.UK** — File your confirmation statement (annually; up to 14 days after the review period ends) | `https://www.gov.uk/guidance/filing-your-companys-confirmation-statement` | High (official) |
| S17 | **GOV.UK (Companies House EPR spec)** — company number: "Give the full 8 digits. If there are leading zeroes, include them" | `https://www.gov.uk/government/publications/organisation-details-how-to-create-your-file-for-extended-producer-responsibility-epr-for-packaging/organisation-details-file-specification-for-extended-producer-responsibility` | High for 8-digit length (official); prefix list Medium (secondary: Inform Direct/Experian/Wikipedia) |
| S18 | **Inform Direct / Experian** — CRN format: always 8 characters (8 digits for E&W, or 2-letter prefix + 6 digits: SC Scotland, NI Northern Ireland, OC LLP E&W, LP limited partnership E&W, FC/RC etc.) | `https://www.informdirect.co.uk/company-records/company-registration-number-crn-what-is-it/` · `https://www.experian.co.uk/blogs/latest-thinking/guide/company-registration-number/` | Medium (reputable secondary; no single official format spec page) |
| S19 | **Xero UK** — Chart of accounts glossary (1000 current account, 1100 accounts receivable, 2000 accounts payable, 2100 VAT liability …) | `https://www.xero.com/uk/glossary/chart-of-accounts/` | High for *software convention* (not statutory) |
| S20 | **QuickBooks** — Chart of accounts numbering (Assets 1000–1999, Liabilities 2000–2999, Equity 3000–3999, Income 4000–4999, Expenses 5000+) | `https://quickbooks.intuit.com/accounting/chart-accounts/` | High for *software convention* (not statutory) |
| S21 | **Statrys** — UK sort code (6 digits) + 8-digit account number route BACS, CHAPS, Faster Payments | `https://statrys.com/blog/what-is-a-sort-code` | High (reputable secondary; rails confirmed by Pay.UK sort-code checker `https://newseventsinsights.wearepay.uk/sort-code-checker/`) |
| S22 | **GoCardless / Stripe** — SEPA status after Brexit (UK remains a SEPA member; GB IBANs valid for EUR transfers; SEPA is not a domestic GBP rail) | `https://gocardless.com/guides/posts/is-uk-still-sepa-country` · `https://stripe.com/resources/more/is-the-uk-a-sepa-country-here-is-what-businesses-need-to-know` | Medium/High (reputable; nuance vs. "UK left SEPA", see §9) |
| S23 | **NatWest Bankline Direct** — ISO 20022 MX formats (incl. CAMT.053) offered to UK corporate banking customers | `https://www.natwest.com/corporates/everyday-banking/bankline-direct/bankline-direct-iso-faqs.html` | Medium (CAMT.053 availability is bank-dependent; CSV universal) |
| S24 | **GOV.UK** — Corporation Tax rates (small profits 19% under £50K; main rate 25% over £250K; marginal relief between) | `https://www.gov.uk/government/publications/rates-and-allowances-corporation-tax/rates-and-allowances-corporation-tax` | High (official) |
| S25 | **HMRC internal manual BIM81210** — accounting date 31 March–5 April treated as tax-year aligned (basis period reform) | `https://www.gov.uk/hmrc-internal-manuals/business-income-manual/bim81210` | High (official manual) |
| S26 | **legislation.gov.uk** — Companies Act 2006 s.442 (accounts filing period: private 9 months, public 6 months after FYE) | `https://www.legislation.gov.uk/ukpga/2006/46/section/442` | High (statute; not re-fetched this session, stable well-known provision) |

### Key legal facts (verified)
- **No statutory chart of accounts.** Unlike LU (PCN 2020) or NL (RGS), UK companies are free to choose any chart; statutory accounts are governed by FRS 102 (small: Section 1A) / FRS 105 (micro) and Companies Act 2006 (S10, S11, S12, S19, S20).
- **UK is not in the EU**: does not participate in EU ViDA; e-invoicing mandate is a UK national design (Peppol-based, from 2029) (S13, S14).
- Tax year runs **6 April → 5 April**; VAT accounting periods are quarterly by default (S5, S15).

## 1. VAT rates & thresholds (2026)

| Item | Value | Source | Confidence |
|---|---|---|---|
| Standard rate | **20%** (since 4 Jan 2011, raised from 17.5%) | S1 | **High** |
| Reduced rate | **5%** (e.g. children's car seats, home energy) | S1 | High |
| Zero rate | **0%** (e.g. most food, children's clothes, books) | S1 | High |
| Exempt | Financial services, insurance, education, property transactions, postage stamps — no VAT charged, no input reclaim (mostly) | S1 | High |
| Registration threshold | **£90,000** taxable turnover (12-month rolling or expected next 30 days) | S2, S3 | High |
| Deregistration threshold | **£88,000** (optional cancellation once below; effective 1 Apr 2024, from £83,000) | S2, S3 | High |
| NI acquisitions threshold | £90,000 (goods into NI from EU) | S2 | High |
| Corporation Tax 2026 | Small profits **19%** (profits < £50K) · main rate **25%** (profits > £250K) · marginal relief £50K–£250K | S24 | High (context) |

## 2. VAT schemes relevant to SMEs

All three mainstream HMRC schemes are relevant for SMEs; thresholds verified (S2):

| Scheme | Join | Leave | Notes | Confidence |
|---|---|---|---|---|
| **Flat Rate Scheme** | turnover ≤ £150,000 | > £230,000 | Pay a fixed % of turnover instead of output−input; sector flat rates; **limited cost trader pays 16.5%** (goods costs < 2% of turnover or < £1,000/yr). 1% discount in first year (sole traders). | High (S2, S4) |
| **Cash Accounting** | ≤ £1.35M | > £1.6M | Account for VAT when paid/received, not invoiced — cash-flow benefit. | High (S2) |
| **Annual Accounting** | ≤ £1.35M | > £1.6M | One VAT return per year, 9 monthly instalments (or 3 quarterly) + balancing payment. | High (S2) |

- Default VAT filing: **quarterly** returns ("every 3 months"), due online **1 calendar month + 7 days** after the accounting period ends (S5). Return layout (9-box VAT return) is a B-milestone for bukio — like LU, do not register `tax.returnLayout` until the UK return engine exists.

## 3. Company identifiers

| Identifier | Format | Source | Confidence |
|---|---|---|---|
| **Companies House company number (CRN)** | **8 characters**: 8 digits (E&W — typically leading 0/1) or 2-letter prefix + 6 digits. Prefixes: `SC` Scotland, `NI` Northern Ireland, `OC` LLP (E&W), `LP` limited partnership (E&W), `FC`/`RC` overseas/community-interest variants. Keep leading zeros (e.g. `00123456`). | S17, S18 | High for 8-char/leading-zeros (official spec); Medium for full prefix list (secondary sources only) |
| **VAT registration number** | `GB` + **9 digits** (`GB123456789`); **`XI` prefix for Northern Ireland** businesses (same digits, e.g. `XI 123456789`) for EU trade. | S6 | High |
| **UTR (Unique Taxpayer Reference)** | **10 digits**, issued by HMRC on Self Assessment registration or company incorporation; used for Self Assessment and Corporation Tax. (HMRC's design pattern allows entry of 10 or 13 digits — standard is 10.) | S7 | High |
| Peppol scheme identifier | Not yet assigned for the UK mandate (expected with the 2026 roadmap; Peppol participant IDs use scheme IDs per country). **Leave `peppolSchemeId` null / unverified.** | S13, S14 | **Unverified — flag** |

## 4. Legal forms (common SME forms)

| Form | Suffix | Notes | Source | Confidence |
|---|---|---|---|---|
| **Private limited company** | Ltd | Most common SME incorporated form; limited liability; Corporation Tax; CRN 8 digits | S8 | High |
| **Sole trader** | — | Simplest form; unlimited liability; Self Assessment + MTD for Income Tax; no Companies House filing | S8, S15 | High |
| **Business partnership** | — | General partnership (unincorporated); partners taxed via SA; HMRC partnership return | S8 | High |
| **Limited liability partnership** | LLP | Incorporated, members have limited liability; CRN prefix `OC`; files accounts + confirmation statement | S8, S18 | High (form) / Medium (prefix) |
| **Community interest company** | CIC | Social enterprise; limited by shares or guarantee; regulated by the CIC Regulator | S8 | High (existence on GOV.UK "social enterprise") |
| **Charity** | — | Registered with the Charity Commission (E&W); charitable company or CIO; accounts per charity SORP | S8 | Medium (not detailed on the cited page) |

## 5. Statutory accounts (Companies House filing)

**Size thresholds** (any 2 of the 3 criteria; S10):

| Regime | Turnover | Balance sheet | Employees | Accounting basis | Filed to Companies House |
|---|---|---|---|---|---|
| **Micro-entity** | ≤ £1M | ≤ £500K | ≤ 10 | **FRS 105** | **Balance sheet only** (abridged, less info) + notes; P&L prepared but **not filed** |
| **Small company** | ≤ £15M | ≤ £7.5M | ≤ 50 | **FRS 102 Section 1A** | Full or **abridged** balance sheet + notes; P&L and directors' report **optional to file** (filleting); audit exemption |

- **Abridged accounts** require agreement of all members; balance sheet must be signed by a director with the director's name printed (S10, S12).
- Balance-sheet statements required (examples, S12): "prepared in accordance with the provisions applicable to companies subject to the small companies regime"; abridged: "All of the members have consented to the preparation of abridged accounts in accordance with Section 444(2A) of the Companies Act 2006".
- **Deadlines** (S9, S26): annual accounts to Companies House **9 months** after FYE (private; **6 months** for public companies — CA 2006 s.442); Corporation Tax payment **9 months + 1 day** after accounting period end; CT600 return **12 months** after period end; **confirmation statement (CS01) annually, within 14 days** after the review period ends, £50 fee (S16).
- Statutory accounts to HMRC as part of the Company Tax Return (CT600) (S9).
- UK GAAP hierarchy: FRS 102 (full), FRS 102 §1A (small), FRS 105 (micro) — set by the **FRC** (S11, S12).

## 6. e-Invoicing & Making Tax Digital (status as of 2026)

- **No e-invoicing mandate in force as of 2026** — but **mandatory e-invoicing for ALL VAT invoices (B2B + B2G) announced for 2029** at Budget 2025 (consultation response, Nov 2025); implementation roadmap to be published at **Budget 2026** (S13). **High** confidence, with the 2029 date flagged as a near-term change for the profile.
- **Peppol named as the core interoperability network** for the UK regime (Tax Update 2026, June 2026) (S14). EN 16931/Peppol BIS alignment expected; not yet spec'd — keep `documents.eInvoicing` at `'peppol-bis-3.0'`-style readiness but mark future-dated.
- **UK does NOT participate in EU ViDA** (EU initiative; UK outside the EU) — confirmed implicitly by the consultation glossary describing ViDA as an EU measure and the UK designing its own regime (S13). High.
- **MTD for VAT**: VAT returns already fully digital via MTD (mandatory since 1 Apr 2022) — software-to-HMRC API filing; relevant for bukio's VAT engine design. Medium-High (well-established; see S5 for the quarterly return).
- **MTD for Income Tax** timeline (qualifying self-employment/property income; S15, updated 26 Mar 2026):
  - from **6 April 2026**: > £50,000 (2024/25 tax year)
  - from **6 April 2027**: > £30,000 (2025/26)
  - from **6 April 2028**: > £20,000 (2026/27)
  - Partnerships: timeline to be set later. Exemptions exist (digitally excluded etc.).

## 7. Chart of accounts (no statutory chart — dominant software convention)

The UK has **no statutorily mandated chart** (contrast LU PCN / NL RGS). The dominant SME convention is the **4-digit QuickBooks/Xero numbering**: **1000s assets, 2000s liabilities, 3000s equity, 4000s income, 5000s+ expenses** (S19, S20 — verified as the actual Xero/QuickBooks UK numbering, e.g. Xero: 1000 business current account, 1100 accounts receivable, 2000 accounts payable, 2100 VAT liability). Confidence: **High that this is the dominant convention; explicitly NOT statutory**.

### Recommended 40-account default chart for `gb.js` (QuickBooks/Xero-style, French-style labels in English)

| Code | Name (en-GB label) | Type | Normal balance |
|---|---|---|---|
| 1000 | Bank — current account | asset | debit |
| 1010 | Cash (petty cash) | asset | debit |
| 1100 | Trade debtors (accounts receivable) | asset | debit |
| 1200 | Prepayments | asset | debit |
| 1300 | Stock / inventory | asset | debit |
| 1400 | Other debtors | asset | debit |
| 1500 | Office equipment (fixed asset) | asset | debit |
| 1600 | Accumulated depreciation — office equipment | asset (contra) | credit |
| 1700 | Motor vehicles (fixed asset) | asset | debit |
| 1800 | Accumulated depreciation — motor vehicles | asset (contra) | credit |
| 2000 | Trade creditors (accounts payable) | liability | credit |
| 2100 | VAT control (output VAT) | liability | credit |
| 2110 | VAT input (reclaimable VAT) | asset | debit |
| 2120 | VAT — balance due to/from HMRC (settlement) | liability | credit |
| 2200 | PAYE / National Insurance control | liability | credit |
| 2300 | Accruals | liability | credit |
| 2400 | Directors' loan account | liability | credit |
| 2500 | Corporation tax payable | liability | credit |
| 2600 | Other creditors | liability | credit |
| 3000 | Called-up share capital | equity | credit |
| 3100 | Share premium | equity | credit |
| 3200 | Profit and loss account (retained earnings) | equity | credit |
| 3300 | Profit / (loss) for the year | equity | credit |
| 4000 | Sales — goods | income | credit |
| 4100 | Sales — services | income | credit |
| 4200 | Other income | income | credit |
| 5000 | Purchases (cost of goods) | expense | debit |
| 5100 | Cost of sales — stock movements | expense | debit |
| 6000 | Wages and salaries | expense | debit |
| 6100 | Rent and rates | expense | debit |
| 6200 | Utilities (electricity, gas, water) | expense | debit |
| 6300 | Telephone and internet | expense | debit |
| 6400 | Insurance | expense | debit |
| 6500 | Motor and travel expenses | expense | debit |
| 6600 | Repairs and maintenance | expense | debit |
| 6700 | Printing, postage and stationery | expense | debit |
| 6800 | Professional fees (accountant, legal) | expense | debit |
| 6900 | Advertising and marketing | expense | debit |
| 7000 | Bank charges and interest | expense | debit |
| 7100 | Depreciation | expense | debit |
| 7200 | Bad debts written off | expense | debit |
| 7300 | Miscellaneous expenses | expense | debit |

(40 accounts: 9 asset + 1 contra-asset pair counted as listed; adjusts to the 30–40 target. Equity statement on a UK small-company balance sheet shows "Capital and reserves: Called up share capital / Profit and loss account" — 3000/3200 map to those lines; 3300 is the current-year result before appropriation.)

## 8. Fiscal year end & filing calendar

- **Any year end is allowed** for companies (choose at incorporation; CH "accounting reference date"). **31 March is a common choice** for both companies and sole traders because it aligns with the tax year (HMRC treats 31 March–5 April as tax-year-aligned under basis period reform; 31 December also common). Confidence: **High** for "any year end allowed + 31 Mar/5 Apr aligned" (S25); **Medium** for "31 March most common" (industry practice, not statutorily measured).
- **Companies House accounts**: 9 months after FYE (private) / 6 months (public) (S9, S26).
- **CT600 Corporation Tax return**: 12 months after accounting period end; CT payment 9 months + 1 day (S9).
- **VAT returns**: quarterly by default; annual accounting scheme option (S5, S2).
- **Confirmation statement**: annually, within 14 days of the review period end (S16).
- **Self Assessment / MTD for Income Tax**: tax year 6 April–5 April; digital quarterly updates + final declaration from April 2026 (thresholds in §6) (S15).
- bukio default: `defaultFiscalYearEnd: '03-31'` — sensible for GB (tax-year alignment; Medium-High confidence as a *default*, any date legal).

## 9. Banking

| Item | Fact | Source | Confidence |
|---|---|---|---|
| Account identifiers | **Sort code: 6 digits** (format XX-XX-XX) + **account number: 8 digits** — domestic identifier; **NOT IBAN-based** for domestic payments | S21 | High |
| Domestic payment rails | **Faster Payments** (near-instant, 24/7, up to £1M/tx), **BACS** (batch, 2–3 working days, Direct Debit + Direct Credit), **CHAPS** (same-day, high-value, ~£25–30 fee) — all routed on sort code + account number | S21 | High |
| IBAN | UK accounts **do have IBANs** (`GB` + 2 check digits + 4-letter bank code + sort code + account number) — used for **international** transfers, not domestic | S22 | High |
| SEPA | **Nuance vs "UK left SEPA":** the UK remains a **SEPA member** post-Brexit — GB IBANs are still valid for EUR SEPA transfers to the EU. But **SEPA is NOT a domestic UK rail**: domestic GBP payments never use SEPA/IBAN. So "SEPA not applicable domestically" is correct; "UK left SEPA" is imprecise. | S22 | Medium/High |
| Bank statement formats | **CAMT.053 (ISO 20022 XML)** available from UK corporate banks (e.g. NatWest Bankline Direct); **CSV is the universal SME format**; MT940 legacy | S23 | Medium (availability varies by bank) |
| bukio mapping | `accountNumber: { kind: 'sort-code-account' }` — sort code (6) + account number (8); **do not seed an IBAN-based account model**; exchange supports CAMT.053 + CSV import | S21, S23 | High |

## 10. Currency & locale

| Item | Value | Confidence |
|---|---|---|
| Currency | **GBP (£)**, ISO 4217 `GBP`; minor unit **pence**; 2 decimal places; integer pence in the engine (bukio convention) | High (ISO 4217 standard) |
| Locale | `en-GB` (en_GB) | High |
| Date format | **dd/mm/yyyy** | High |
| Note | VAT amounts on invoices must show net, VAT rate/amount, gross; VAT fraction for flat-rate invoicing | High (S4 flat-rate mechanics) |

## 11. Recommendations for `gb.js` (mapping to the LU profile shape)

- `meta`: country `'GB'`, baseCurrency `'GBP'`, locale `'en-GB'`, legalForms `['sole-trader','private-limited-company','partnership','llp','cic','charity']`, defaultFiscalYearEnd `'03-31'`.
- `identifiers`: companyIdLabel `'company_number'` (8 chars, `/^(?:[A-Z]{2})?\d{6}$/i`-style advisory validation — keep simple: 8 chars), vatIdLabel `'vat_number'` with format `/^(GB|XI)\d{9}$/i`, UTR as a 10-digit field, accountNumber `{ kind: 'sort-code-account' }` (6+8).
- `tax`: system `'vat'`; standardRateBp `2000`; codes for 20 / 5 / 0 / exempt / reverse-charge / margin; smallBusinessScheme: flat-rate option (threshold £150K); VAT control accounts 2100/2110/2120.
- `reporting.taxonomy`: `null` (no statutory taxonomy — chart is software convention).
- `documents.eInvoicing`: note Peppol 2029 mandate; **not yet implementable** (no UK Peppol scheme ID assigned) — leave unregistered until the 2026 roadmap.
- `compliance.filingTypes`: confirmation statement (annual, +14 days), annual accounts (9 months), CT600 (12 months) — calendar-relevant.
- **Do NOT register** `tax.returnLayout` (VAT 9-box return engine is a B-milestone) or statutory-accounts layout (FRS 102/105 XBRL iXBRL filing is a B-milestone) — fail loudly like LU until built.

## 12. Unverifiable / flagged items

1. **Peppol scheme ID for the UK** — no identifier published yet (roadmap due Budget 2026). **Unverifiable as of 2026-08-15.**
2. **Exact Xero/QuickBooks default account codes** — the *ranges* (1000/1100/2000/2100/4000/5000) are verified (S19, S20) but the full 40-account list above is bukio's design following that convention; individual account *numbers* in Xero/QuickBooks vary slightly by product/edition. Confidence: High (convention), Medium (specific codes).
3. **"31 March is the most common company year end"** — well-supported practice but no official statistic verified; any year end is legal (S25).
4. **Public company 6-month deadline** — CA 2006 s.442 (S26); not re-fetched this session (stable provision, widely documented). High confidence, cite legislation.gov.uk.
5. **CAMT.053 availability** — confirmed for major UK corporate banks (NatWest example, S23); smaller/retail banks vary — CSV is the safe universal import.
6. **XI-prefix digit identity** — GOV.UK example implies same digits with different prefix (S6); EU trade vs domestic prefix usage rules have edge cases not fully enumerated here.
7. **MTD for VAT mandatory-from date (1 Apr 2022)** — stated from knowledge, not re-verified against a fetched page this session; the current quarterly-digital status is not in doubt (S5), the exact 2022 start date should be confirmed if cited in code comments.
