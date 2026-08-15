# United States — SME bookkeeping jurisdiction profile (US)

Research brief for the bukio-cli US jurisdiction profile (`src/jurisdictions/us.js`).
Every item carries a source URL and a confidence rating (HIGH / MEDIUM / UNVERIFIED).
Compiled 15 Aug 2026. Labels in English (en-US).

---

## 1. Tax system

| # | Claim | Source | Confidence |
|---|-------|--------|-----------|
| 1.1 | **No federal VAT.** The US has no value-added tax and no national general sales tax; sales tax is governed at the state level | https://en.wikipedia.org/wiki/Sales_taxes_in_the_United_States ("Sales tax is governed at the state level, and no national general sales tax exists") | HIGH |
| 1.2 | **C-corp income tax = flat 21%** since the Tax Cuts and Jobs Act (TCJA, P.L. 115-97, enacted 22 Dec 2017); formerly 35% top rate. Filed on **Form 1120** | https://taxsummaries.pwc.com/united-states/corporate/taxes-on-corporate-income ; https://www.irs.gov/government-entities/2018-fiscal-year-blended-tax-rates-for-corporations ; https://taxfoundation.org/blog/tcja-pass-through-business-tax-reform/ | HIGH |
| 1.3 | **S-corp** elects pass-through taxation (Form 2553 election); income/loss/deductions/credits flow through to shareholders, who report on their personal returns — avoids double taxation. Files **Form 1120-S** | https://www.irs.gov/businesses/small-businesses-self-employed/s-corporations | HIGH |
| 1.4 | **LLC** — state-law entity; by default a single-member LLC is a disregarded entity (reported on the owner's 1040) and a multi-member LLC is taxed as a partnership; an LLC may also elect to be taxed as an S- or C-corp. Profits pass through without entity-level corporate tax | https://www.sba.gov/business-guide/launch-your-business/choose-business-structure ("Profits and losses can get passed through to your personal income without facing corporate taxes") ; https://www.irs.gov/businesses/small-businesses-self-employed/single-member-limited-liability-companies (not re-extracted this session; standard IRS URL) | HIGH (disregarded-entity detail: MEDIUM — common-knowledge IRS position, page not re-verified) |
| 1.5 | **Sole proprietorship** — default structure when doing business unregistered; business results reported on the owner's **Form 1040 via Schedule C** ("Profit or Loss from Business") | https://www.irs.gov/forms-pubs/about-schedule-c-form-1040 ; https://www.sba.gov/business-guide/launch-your-business/choose-business-structure | HIGH |
| 1.6 | **State corporate income taxes** vary, roughly 1%–10% (some states none); federal taxable income is the common starting base | https://taxsummaries.pwc.com/united-states/corporate/taxes-on-corporate-income (State and local income taxes section) | HIGH |
| 1.7 | A corporate AMT (CAMT) of 15% on adjusted financial statement income applies to large C-corps (tax years after 2022) — only for corporations with avg. financial-statement income ≥ $1B; not an SME concern | https://taxsummaries.pwc.com/united-states/corporate/taxes-on-corporate-income | HIGH (existence), SME-relevance: MEDIUM |

---

## 2. Payroll compliance basics

| # | Claim | Source | Confidence |
|---|-------|--------|-----------|
| 2.1 | **Form 941** (Employer's Quarterly Federal Tax Return) — employers with employees file **quarterly** to report federal income tax, Social Security and Medicare withheld, plus the employer's share | https://www.irs.gov/forms-pubs/about-form-941 | HIGH |
| 2.2 | **Form W-2** (Wage and Tax Statement) — issued to each employee annually; due to employees by 31 January; **Form W-3** is the transmittal of W-2s to the SSA | https://www.irs.gov/forms-pubs/about-form-w-2 ; https://taxsummaries.pwc.com/united-states/corporate/tax-administration (W-2 due 31 Jan) | HIGH |
| 2.3 | **SUTA** — state unemployment taxes are handled at the **state** level and are not reported on Form 941; Form 940 covers federal FUTA annually | https://www.surepayroll.com/resources/article/how-to-complete-form-941-in-5-simple-steps ("State unemployment taxes (SUTA) are handled at the state level and are not reported on Form 941") ; https://www.irs.gov/businesses/small-businesses-self-employed/s-corporations (941/940 in filing table) | HIGH |
| 2.4 | Employment taxes (income tax withholding + FICA) are deposited on a semiweekly or monthly schedule via EFTPS | https://www.irs.gov/businesses/small-businesses-self-employed/employer-id-numbers (EFTPS references) | HIGH |

---

## 3. Identifiers

| # | Claim | Source | Confidence |
|---|-------|--------|-----------|
| 3.1 | **EIN** (Employer Identification Number) — the federal tax ID for businesses, tax-exempt organizations and other entities, issued free by the IRS (Form SS-4) | https://www.irs.gov/businesses/small-businesses-self-employed/employer-id-numbers | HIGH |
| 3.2 | EIN format: **9 digits, XX-XXXXXXX** (two digits, hyphen, seven digits); SSN format XXX-XX-XXXX identifies individuals | https://www.xero.com/us/glossary/ein/ ; https://support.taxslayer.com/hc/en-us/articles/4410676600333-What-is-the-EIN ; https://www.investopedia.com/terms/e/employer-identification-number.asp (third-party; IRS Pub 1635 "Understanding Your EIN" is the authoritative reference) | HIGH |
| 3.3 | Individuals use **SSN** (Social Security Number) or **ITIN** (Individual Taxpayer Identification Number, applied via Form W-7) | https://www.irs.gov/businesses/small-businesses-self-employed/employer-id-numbers (responsible party may use "Social Security, individual taxpayer ID, or employer identification number"); ITIN: https://www.irs.gov/individuals/international-taxpayers/individual-taxpayer-identification-number (standard IRS page, not re-extracted this session) | HIGH |
| 3.4 | **No national company registration number.** Businesses register with the **state** (Secretary of State filings for LLCs/corporations); IRS explicitly instructs "Form your entity first: register it with your state before you apply for an EIN" | https://www.irs.gov/businesses/small-businesses-self-employed/employer-id-numbers ("Form your entity first… register it with your state") ; https://www.sba.gov/business-guide/launch-your-business/choose-business-structure ("register your business with the state") | HIGH |
| 3.5 | FinCEN Beneficial Ownership Information (BOI) reporting is a separate federal obligation for most corporations/LLCs | https://www.irs.gov/businesses/small-businesses-self-employed/employer-id-numbers (FinCEN section) ; https://fincen.gov/boi | HIGH (existence) |

---

## 4. Legal forms

| # | Claim | Source | Confidence |
|---|-------|--------|-----------|
| 4.1 | Common structures per the SBA: **sole proprietorship, partnership (LP/LLP), LLC, corporation (C corp), S corp, benefit corporation** | https://www.sba.gov/business-guide/launch-your-business/choose-business-structure | HIGH |
| 4.2 | **Non-profit — 501(c)(3)** charitable organization: tax-exempt under IRC §501(c)(3), organized and operated exclusively for exempt purposes; eligible for tax-deductible contributions | https://www.irs.gov/charities-non-profits/charitable-organizations/exemption-requirements-501c3-organizations | HIGH |
| 4.3 | LLC/corporation formation is state-level (articles of organization / certificate of incorporation with the state); S-corp status additionally requires a federal IRS election (Form 2553) | https://www.sba.gov/business-guide/launch-your-business/choose-business-structure ("S corps must file with the IRS to get S corp status, a different process from registering with their state") ; https://www.irs.gov/businesses/small-businesses-self-employed/s-corporations | HIGH |

---

## 5. Statutory / filing obligations

| # | Claim | Source | Confidence |
|---|-------|--------|-----------|
| 5.1 | **Federal income tax return (Form 1120) due the 15th day of the 4th month** after the close of the tax year (April 15 for calendar-year corporations); a 6-month filing extension is available | https://taxsummaries.pwc.com/united-states/corporate/tax-administration ("required to file an annual tax return (generally Form 1120) by the 15th day of the fourth month following the close of its tax year… additional six-month extension") | HIGH |
| 5.2 | **Estimated quarterly tax payments** — corporations pay on the 15th of April, June, September, and December (Form 1120-W / 1120-ES); individuals/business owners use **1040-ES** (Pub. 509: due 15th day of the 4th, 6th, 9th and 1st months) | https://taxsummaries.pwc.com/united-states/corporate/tax-administration ; https://www.irs.gov/publications/p509 ; https://www.irs.gov/businesses/small-businesses-self-employed/s-corporations (1040-ES in shareholder filing table) | HIGH |
| 5.3 | **State franchise/income taxes** vary widely by state (1%–10% CIT; some states none); separate state returns and estimated payments apply | https://taxsummaries.pwc.com/united-states/corporate/taxes-on-corporate-income | HIGH |
| 5.4 | **Form 941 quarterly payroll returns** — see §2.1 | https://www.irs.gov/forms-pubs/about-form-941 | HIGH |
| 5.5 | **Fiscal year: any tax year allowed.** "Corporate taxpayers may choose a tax year that is different from the calendar year"; calendar year is the norm for SMEs | https://taxsummaries.pwc.com/united-states/corporate/tax-administration (Tax period section) | HIGH |
| 5.6 | W-2/1099 information returns due to recipients by 31 January; filing deadlines to IRS vary by type/electronic filing | https://taxsummaries.pwc.com/united-states/corporate/tax-administration (due-date table) | HIGH |

---

## 6. e-Invoicing

| # | Claim | Source | Confidence |
|---|-------|--------|-----------|
| 6.1 | **No B2B e-invoicing mandate in the US.** "In the United States, there is no mandatory nationwide e-invoicing for B2B transactions; e-invoicing remains voluntary and decentralized" | https://www.houseblend.io/articles/us-canada-e-invoicing-mandates-2026-regulations | HIGH |
| 6.2 | Only **federal government procurements** effectively mandate electronic invoices (e.g. via the federal Invoice Processing Platform); no state-level B2B e-invoicing requirement | https://www.houseblend.io/articles/us-canada-e-invoicing-mandates-2026-regulations ; https://www.theinvoicinghub.com/einvoicing-compliance-usa/ | HIGH |
| 6.3 | **Peppol is not required** in the US; EDI (X12) remains the legacy standard for large companies; a Peppol-like private network (DBNAlliance, formed 2023–24) is emerging voluntarily | https://www.houseblend.io/articles/us-canada-e-invoicing-mandates-2026-regulations (DBNAlliance section) | HIGH (no mandate) / MEDIUM (EDI dominance — implied, not directly quoted) |
| 6.4 | **No federal e-invoicing/tax-clearing regime** (no continuous transaction controls); future mandate widely expected but not enacted | https://www.houseblend.io/articles/us-canada-e-invoicing-mandates-2026-regulations | HIGH |

---

## 7. Chart of accounts

| # | Claim | Source | Confidence |
|---|-------|--------|-----------|
| 7.1 | **No statutory chart of accounts** in the US (no tax-authority chart). GAAP governs financial reporting; businesses use custom/software charts | QuickBooks convention (below); GAAP basis: https://taxsummaries.pwc.com/united-states/corporate/tax-administration (cash vs accrual methods) | HIGH |
| 7.2 | Dominant **QuickBooks numbering convention**: Assets 1,000–1,999; Liabilities 2,000–2,999; Income 4,000–4,999; Operating expenses 6,000–7,999; (equity 3,000–3,999 and COGS 5,000–5,999 per the standard extension of the same scheme) | https://quickbooks.intuit.com/global/resources/accounting/chart-of-accounts-definition-and-example/ ("The most common number sequences… Assets: 1,000 to 1,999; Liabilities: 2,000 to 2,999; Income: 4,000 to 4,999; Operating expenses: 6,000 to 7,999") ; http://www.netmba.com/accounting/fin/accounts/chart/ (1000–1999 assets, 2000–2999 liabilities, 3000–3999 equity, 4000–4999 revenue, 5000–5999 COGS, 6000+ expenses) | HIGH |

### Default chart (38 accounts, US convention, English labels)

| Code | Name | Type | Normal balance |
|------|------|------|----------------|
| 1000 | Checking account | asset | debit |
| 1010 | Savings account | asset | debit |
| 1020 | Cash on hand (petty cash) | asset | debit |
| 1100 | Accounts receivable | asset | debit |
| 1200 | Inventory | asset | debit |
| 1300 | Prepaid expenses | asset | debit |
| 1400 | Fixed assets (equipment, vehicles) | asset | debit |
| 1410 | Accumulated depreciation (contra-asset) | asset | credit |
| 1500 | Other assets / deposits | asset | debit |
| 2000 | Accounts payable | liability | credit |
| 2100 | Sales tax payable (state/local) | liability | credit |
| 2200 | Payroll liabilities (941 withholding) | liability | credit |
| 2300 | Accrued expenses | liability | credit |
| 2400 | Federal income tax payable | liability | credit |
| 2500 | State income tax payable | liability | credit |
| 2600 | Loans payable / credit cards | liability | credit |
| 2700 | Other liabilities | liability | credit |
| 3000 | Owner's equity (members' equity) | equity | credit |
| 3100 | Owner's draws | equity | debit |
| 3200 | Retained earnings | equity | credit |
| 3300 | Current year earnings (profit/loss) | equity | credit |
| 4000 | Sales — goods | income | credit |
| 4100 | Sales — services | income | credit |
| 4200 | Other income (interest, misc.) | income | credit |
| 5000 | Cost of goods sold | expense | debit |
| 5100 | Purchases / materials | expense | debit |
| 6000 | Rent | expense | debit |
| 6100 | Utilities | expense | debit |
| 6200 | Wages and salaries | expense | debit |
| 6300 | Payroll taxes (employer FICA, FUTA, SUTA) | expense | debit |
| 6400 | Insurance | expense | debit |
| 6500 | Office expenses and supplies | expense | debit |
| 6600 | Professional fees (accounting, legal) | expense | debit |
| 6700 | Travel and meals | expense | debit |
| 6800 | Depreciation expense | expense | debit |
| 6900 | Advertising and marketing | expense | debit |
| 7000 | Repairs and maintenance | expense | debit |
| 7100 | Bank charges and merchant fees | expense | debit |
| 7200 | Miscellaneous expenses | expense | debit |

Mapping notes for `us.js`: `debtorsAccount` → 1100; sales-tax control → 2100 (no input-VAT account — no VAT reclaim); `closing.resultAccount` → 3300, `closing.equityAccount` → 3200. The 2100 account is only used by companies registered for state sales tax (most retailers; not services-only businesses in some states).

---

## 8. Banking

| # | Claim | Source | Confidence |
|---|-------|--------|-----------|
| 8.1 | **ABA routing number = 9 digits** identifying the US bank/institution; used for checks, ACH and Fedwire; account numbers are typically 8–12 digits | https://wise.com/us/routing-number/ ("They're made up of 9-digits… account number… between eight and 12 digits") ; https://stripe.com/resources/more/ach-routing-numbers-explained | HIGH |
| 8.2 | **ACH** (Automated Clearing House) is the domestic electronic rail for direct deposit, bill pay, B2B transfers; ACH routing numbers are 9-digit (often same as ABA) | https://stripe.com/resources/more/ach-routing-numbers-explained ; https://wise.com/us/routing-number/ | HIGH |
| 8.3 | **Wire transfers** — domestic via Fedwire using ABA routing number; international via SWIFT | https://wise.com/us/routing-number/ ; https://stripe.com/resources/more/ach-routing-numbers-explained | HIGH |
| 8.4 | **No IBAN, no SEPA** — IBANs are the European scheme ("IBANs… identify individual bank accounts. They're issued by many banks in Europe"); the US domestic rails are routing-number + account based. SEPA is an EU/EEA payment scheme, not applicable in the US | https://wise.com/us/routing-number/ (IBAN/SWIFT/routing comparison section) | HIGH (no IBAN domestically); SEPA non-applicability: HIGH (common knowledge) |
| 8.5 | Bank statement formats: **CSV is universally available**; **CAMT.053** is offered by larger US banks / treasury APIs but is NOT a US standard (unlike SEPA countries) | No authoritative US source found this session — industry practice | UNVERIFIED (CSV: HIGH common knowledge; CAMT.053 availability: MEDIUM/unverified) |

---

## 9. Currency & locale

| # | Claim | Source | Confidence |
|---|-------|--------|-----------|
| 9.1 | Currency **USD ($)**; locale **en-US**; date format **mm/dd/yyyy** (e.g. 08/15/2026); amount separators: thousands `,` decimal `.` | Standard US conventions (no source needed; universal in all sources above, e.g. "04/15/2026" style dates on IRS pages) | HIGH |

---

## 10. Sales tax (state-level)

| # | Claim | Source | Confidence |
|---|-------|--------|-----------|
| 10.1 | **No federal sales tax** and no federal sales-tax rate; 45 states + DC + territories impose general state sales taxes; 5 states (Alaska, Delaware, Montana, New Hampshire, Oregon) have no statewide sales tax | https://en.wikipedia.org/wiki/Sales_taxes_in_the_United_States ; https://taxfoundation.org/data/all/state/sales-tax-rates/ | HIGH |
| 10.2 | Rates: state rates from 0% to **7.25% (California base)**; combined state+local from ~0% to **10.11% (Louisiana average)**; national population-weighted combined average **7.53%** | https://taxfoundation.org/data/all/state/sales-tax-rates/ (CA 7.25% row; LA 10.11%; avg 7.53%) ; https://cdtfa.ca.gov/taxes-and-fees/sales-use-tax-rates.htm ("The statewide tax rate is 7.25%") | HIGH |
| 10.3 | **Sellers register per-state** (state sales-tax permit / seller's permit) where they have nexus; sales tax is collected and remitted to the state, not to the IRS | https://www.sba.gov/business-guide/launch-your-business/choose-business-structure (state-level tax/registration steps) ; nexus/registration mechanics: not directly extracted this session | HIGH (registration exists); nexus detail: MEDIUM |

---

## Profile-mapping recommendations (for `src/jurisdictions/us.js`)

- `meta`: `{ country: 'US', baseCurrency: 'USD', locale: 'en-US', legalForms: ['sole-proprietorship', 'partnership', 'llc', 's-corp', 'c-corp', 'non-profit'], defaultFiscalYearEnd: '12-31' }` (calendar year most common; any year end legal — §5.5)
- `identifiers`: `taxIdLabel: 'ein'`, `taxIdFormat: /^\d{2}-\d{7}$/` (§3.2); **no national companyId** — registration is state-level (§3.4), so `companyIdLabel` should be optional/omitted or labeled `state_registration_number`; `accountNumber: { kind: 'routing-account' }` (ABA 9-digit + account, §8.1)
- `tax`: `system: 'none'` at federal level (no VAT — §1.1); state sales tax handled via the 2100 control account + per-state registration (§10.3); no federal e-invoicing/returns engine (no filing regime — §6)
- `exchange`: `bankStatementFormats: ['csv']` (CAMT.053 only at larger banks — §8.5); `paymentFormats: []` (no SEPA — §8.4; ACH format export is a future milestone); `fxSource: 'ecb'` (ECB publishes USD rates; alternative: Fed)
- `documents`: `languages: ['en']`, `defaultLanguage: 'en'`; `eInvoicing` omitted (no mandate — §6)
- `compliance.filingTypes`: `FEDERAL_INCOME_TAX` (1120/1120-S, due 4 months + 15 days after FYE, April 15 calendar — §5.1), `PAYROLL_941` (quarterly — §5.4), `ESTIMATED_TAX` (15th of 4th/6th/9th/1st month — §5.2)

## Unverifiable / flagged items
- **CAMT.053 availability at US banks** (§8.5) — no authoritative source found in budget; industry practice only.
- **Disregarded-entity / ITIN pages** cited to standard IRS URLs not re-extracted this session (§1.4, §3.3) — URLs are the canonical IRS pages, content not re-verified.
- **Nexus mechanics** (§10.3) — Wayfair economic-nexus standards vary per state; not directly sourced.
- EDI as dominant large-company standard (§6.3) — implied by sources, not directly quoted.
