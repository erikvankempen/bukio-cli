# Denmark SME Bookkeeping Profile — Research Brief (dk.js)

Prepared: 2026-08-15 · Scope: SME (ApS/Enkeltmandsvirksomhed) · Currency DKK · Locale da-DK · Date format dd-mm-yyyy
Confidence key: **HIGH** = official/primary source extracted · **MED** = multiple secondary sources agree · **LOW** = convention/domain knowledge, unverified this session

---

## 1. VAT rates, threshold, small-business scheme

| Item | Finding |
|---|---|
| Standard rate | **25%** — highest in the EU; **no reduced rate band** exists in Denmark. A few items are 0% (exports, some newspapers/periodicals; passenger transport rules vary) rather than reduced-rated. |
| Registration threshold | **DKK 50,000** annual taxable turnover in a 12-month period → mandatory VAT registration (voluntary below). |
| Small-business exemption scheme | **None.** No Danish equivalent of the Dutch KOR (Kleineondernemersregeling) or similar SME VAT-exemption scheme; no cash-accounting VAT basis — VAT is on invoice/accrual basis. Flag: "KOR" was not named in any source this session; the inference that it refers to a small-business exemption analogue is from domain knowledge. |

Sources: https://invoxo.eu/learn/country-guides/denmark ("no reduced rates for most goods and services" — wording implies some zero-rated items) · https://www.avalara.com/us/en/vatlive/country-guides/europe/denmark/danish-vat-rates.html · https://www.taxually.com/manuals/denmark · https://taxology.co/blog/vat-denmark/
Confidence: **HIGH** (25% + 50k threshold); zero-rate item list **MED** (secondary sources only). Flag: official skat.dk VAT-rates page failed to extract (http_error) this session.

---

## 2. VAT return (momsangivelse) — frequency & deadline

**⚠️ Correction to assumption "monthly mandatory since 2022 for all": FALSE.** Frequency depends on turnover (official SKAT):

| Frequency | Condition | Deadline (report AND pay, same day) |
|---|---|---|
| **Half-yearly** | VAT-liable revenue < **DKK 5M**/yr AND filed/paid on time previously | **1st** day of the month following the period (1 Sep for H1, 1 Mar for H2; e.g. 2 Sep 2025 when 1st falls on a non-workday — day shifts if deadline lands on weekend/holiday) |
| **Quarterly** | Newly registered (first ~1.5 yrs), or turnover **DKK 5–50M**, or on request | **1st** day of the **3rd** following month (Q2 → 1 Sep, Q3 → 1 Dec) |
| **Monthly** | Turnover **> DKK 50M**/yr, or on request | **25th** of the following month (e.g. Jan 2026 → 25 Feb 2026) |

- "Monthly filing since 2022 for all" is **incorrect** — monthly is only for > DKK 50M or by request; most SMEs file quarterly or half-yearly.
- Newly registered businesses must report **quarterly for at least ~1.5 years** (Marosa).
- Late/non-filing: provisional assessment (foreløbig fastsættelse) + penalty **DKK 1,400/period** (official SKAT). (Marosa quotes DKK 800 + DKK 65 reminder — older figure; use SKAT's 1,400.)
- Filed via **TastSelv Erhverv** (E-tax for businesses). Nil returns still required.
- Excess input VAT is refunded automatically via the company's **NemKonto** (Marosa).

Sources: https://skat.dk/en-us/businesses/vat/deadlines-filing-vat-returns-and-paying-vat (extracted, official) · https://marosavat.com/vat-manual-chapters/denmark-vat-returns (extracted)
Confidence: **HIGH** (deadlines table is verbatim from SKAT).

---

## 3. Identifiers

| Identifier | Format | Notes |
|---|---|---|
| Company number | **CVR number — 8 digits**, last digit is a check digit | Central Business Register (Det Centrale Virksomhedsregister), erhvervsstyrelsen.dk / CVR.dk |
| VAT number (VIES) | **DK + 8-digit CVR** (e.g. DK12345678) — same number as CVR with DK prefix | Confirmed pattern |
| Peppol participant scheme ID | **0184 = DK:CVR (DIGSTORG)** — active scheme for DK | Also legacy/alternative **9902 = DK:CVR**; 0184 is the one used with the CVR in e-invoicing (scheme "0184", identifier = 8-digit CVR) |
| accountNumber kind | **iban** (SEPA; domestic also accepts reg.nr. + account no.) | per task spec, no counter-evidence |

Sources: https://www.oecd.org/content/dam/oecd/en/topics/policy-issue-focus/aeoi/denmark-tin.pdf (8 digits + check digit) · https://peppolgate.eu/peppol-country-codes/ (0184 DK DIGSTORG CVR, Active) · https://developer.vertexinc.com/einvoicing/docs/denmark-messages (@schemeID "0184" = DK CVR-number) · https://en.wikipedia.org/wiki/Central_Business_Register
Confidence: **HIGH** for CVR-8-digits and 0184; VAT-number format **HIGH** (EU-standard, corroborated by Microsoft Dynamics idea: "schemeID 0184 requires DK prefix when using VAT No. as Company Identifier").

---

## 4. Legal forms

| Form | Danish name | Notes |
|---|---|---|
| **ApS** | Anpartsselskab | **The SME standard** — private limited company, limited liability; min. share capital **DKK 40,000** (sources disagree: one says 20,000 — flag; official current minimum is 40,000) |
| A/S | Aktieselskab | Public limited company, min. share capital DKK 400,000 |
| IVS | Iværksætterselskab | **Defunct** — abolished (no new registrations); skip |
| Enkeltmandsvirksomhed | — | Sole trader (personal business, PMV); no min. capital; not a separate legal entity; registers in CVR with CVR/SE number |
| I/S | Interessentskab | Partnership; not a separate legal entity; no min. capital |

Sources: https://www.expanship.com/dk/blog/types-of-companies-in-denmark · https://atrum.dk/registration-of-danish-company-aps (says DKK 20,000 — flagged, likely outdated) · https://businessindenmark.virk.dk/guidance/services-contact-point/Establish-a-business-in-Denmark/ (CVR/SE number for Enkeltmandsvirksomhed)
Confidence: **HIGH** on forms; share-capital figure **MED** (conflicting secondary sources — verify current ApS minimum before shipping).

---

## 5. Chart of accounts (kontoplan)

**Key structural finding — the clean "1xxx assets / 2xxx liabilities / 3xxx equity / 4xxx income / 5xxx costs" scheme is NOT how e-conomic or Dinero number their default charts:**
- **Dinero**: income (omsætning) accounts live in **1000–1999** (e.g. default account "1000 – Salg af varer/ydelser m/moms"); costs follow in higher ranges. Source: https://dinero.dk/support/kontoplan/ (extracted).
- **e-conomic** ("Selskab" chart): direct-cost accounts at **1324** ("Direkte omkostninger varer u/moms 3. lande") / **1325** (ydelser) → so 13xx holds costs, not assets. Source: https://www.e-conomic.dk/support/artikler/momsopsaetning-i-e-conomic-overblik (extracted).
- **Erhvervsstyrelsen official "Fælles offentlig standardkontoplan"** exists, is built from the annual-report schedule (årsregnskabsloven Bilag 2, skema 1 & 3), is distributed as **JSON on GitLab** (git.erst.dk/standard-filformater), and is **required for SAF-T mapping** under the new bookkeeping act; updated 1 Dec 2025, in force from the next accounting-period start in 2026 (calendar-year companies: **1 Jan 2026**). Companies are NOT obliged to use all its accounts — mapping internal chart → official chart is the compliance path. Sources: https://erhvervsstyrelsen.dk/standardkontoplan-saf-t (extracted, official) · https://learn.microsoft.com/en-us/dynamics365/business-central/localfunctionality/denmark/how-to-set-up-standard-coa · https://invoicedataextraction.com/blog/denmark-digital-bookkeeping-act-bogforingsloven

**Recommendation for dk.js**: seed the chart with the parent's 1xxx/2xxx/3xxx/4xxx/5xxx draft (consistent with the official Standardkontoplan's balance/equity/income/cost structure and with årsregnskabsloven skema layout), and add a SAF-T/official-chart mapping hook. Exact account codes below are a **draft convention — MED confidence, NOT verified against e-conomic's actual "Selskab" kontoplan CSV** (e-conomic publishes charts as images/login-gated downloads; exact codes could not be confirmed this session).

### Draft default chart (40 accounts, e-conomic/Standardkontoplan-style)

| Code | Danish label | English | Type | Normal balance |
|---|---|---|---|---|
| 1010 | Kontantkasse | Cash | Asset | Debit |
| 1110 | Bank | Bank account | Asset | Debit |
| 1210 | Tilgodehavender fra salg (Debitorer) | Trade debtors | Asset | Debit |
| 1310 | Debitorer | Trade receivables (alt. naming) | Asset | Debit |
| 1410 | Varebeholdning | Inventory | Asset | Debit |
| 1510 | Materielle anlægsaktiver | Tangible fixed assets | Asset | Debit |
| 1520 | Indretning lejede lokaler | Leasehold improvements | Asset | Debit |
| 1620 | Akkumulerede afskrivninger | Accumulated depreciation (contra) | Asset (contra) | Credit |
| 2110 | Kreditorer | Trade creditors | Liability | Credit |
| 2210 | Skyldig A-skat og AM-bidrag | Withheld payroll taxes | Liability | Credit |
| 2330 | Skyldige lønninger | Payroll payable | Liability | Credit |
| 2380 | Mellemregning ejer | Owner current account (sole trader) | Liability/Equity | Credit |
| 2710 | Salgsmoms | Output VAT | Liability | Credit |
| 2720 | Købsmoms | Input VAT | Asset | Debit |
| 2730 | Afregning af moms | VAT settlement | Liability (clearing) | Credit |
| 2810 | Skyldig selskabsskat | Corporate income tax payable | Liability | Credit |
| 2910 | Hensatte forpligtelser | Provisions | Liability | Credit |
| 3100 | Anpartskapital | Share capital | Equity | Credit |
| 3120 | Overført resultat | Retained earnings | Equity | Credit |
| 3990 | Årets resultat | Result for the year | Equity (P&L clearing) | Credit |
| 4100 | Salg af varer | Sales of goods | Income | Credit |
| 4200 | Salg af ydelser | Sales of services | Income | Credit |
| 4400 | Salg u/moms | Sales exempt/without VAT | Income | Credit |
| 5110 | Varekøb | Purchases of goods | Expense | Debit |
| 5210 | Lønninger | Wages | Expense | Debit |
| 5220 | ATP og øvrige personaleomk. | Staff costs | Expense | Debit |
| 5310 | Pensionsbidrag | Pension contributions | Expense | Debit |
| 5510 | Husleje og lokaleomkostninger | Rent & premises | Expense | Debit |
| 5610 | Vedligeholdelse | Maintenance | Expense | Debit |
| 5710 | El, vand og varme | Utilities | Expense | Debit |
| 5810 | Kontorhold | Office supplies | Expense | Debit |
| 5820 | Telefoni og internet | Phone & internet | Expense | Debit |
| 5910 | Porto og fragt | Postage & freight | Expense | Debit |
| 6110 | Brændstof og bilomkostninger | Fuel & vehicle | Expense | Debit |
| 6210 | Rejseomkostninger | Travel | Expense | Debit |
| 6310 | Repræsentation | Representation/entertainment | Expense | Debit |
| 6410 | Forsikringer | Insurance | Expense | Debit |
| 6510 | IT og software | IT & software | Expense | Debit |
| 6610 | Revisor og advokat | Audit & legal | Expense | Debit |
| 6710 | Markedsføring og annoncer | Marketing & advertising | Expense | Debit |
| 6810 | Afskrivninger | Depreciation | Expense | Debit |
| 6910 | Renteudgifter og gebyrer | Interest & bank fees | Expense | Debit |
| 6990 | Øvrige finansielle omkostninger | Other financial costs | Expense | Debit |

Sources for structure: https://erhvervsstyrelsen.dk/standardkontoplan-saf-t · https://dinero.dk/support/kontoplan/ · https://www.e-conomic.dk/support/artikler/momsopsaetning-i-e-conomic-overblik · https://www.e-conomic.dk/support/artikler/oversigt-over-tilgaengelige-standardkontoplaner-i-e-conomic
Confidence: **MED** (structure) / **LOW–MED** (individual codes; unverified against vendor CSVs — verify before shipping).

---

## 6. VAT control accounts

| Code | Danish label | Function | Normal balance |
|---|---|---|---|
| **2710** | **Salgsmoms** | Output VAT (moms af salg) | Credit |
| **2720** | **Købsmoms** | Input VAT (moms af køb) | Debit |
| **2730** | **Afregning af moms** (også: Momsafregning) | Settlement account cleared when VAT is paid/refunded via skattekonto | Credit (clearing) |

- These 27xx codes match the parent's expectation and the e-conomic 27xx VAT area, but **exact codes could NOT be verified from primary docs** (e-conomic publishes charts as images/login downloads; no text listing found). One forum source references account **6917** for booking paid VAT in e-conomic's standard chart — indicating settlement handling varies by chart; flag before relying on 27xx.
- The Danish VAT return (momsangivelse) itself distinguishes **Salgsmoms** (output) vs **Købsmoms** (input/deductible) — the terminology is statutory.

Sources: https://www.e-conomic.dk/support/artikler/momsopsaetning-i-e-conomic-overblik (27xx area implied, codes not listed) · https://www.amino.dk/forums/t/41248.aspx (6917 workaround — low authority) · https://skat.dk/en-us/businesses/vat/deadlines-filing-vat-returns-and-paying-vat (momsangivelse terminology)
Confidence: **MED** (naming/function), **LOW** (exact codes unverified).

---

## 7. Statutory accounts (årsrapport)

- Annual report (årsrapport) per **årsregnskabsloven**; **class B (small)** companies file with **Erhvervsstyrelsen** within **5 months of FYE** (multiple advisory sources agree; the general deadline for most ApS/A/S). Deadline of 6 months applies to larger/other classes (e.g. class C large = 4 months after FYE; listed = 3). Flag: 5 months for class B is corroborated by 4+ independent advisory sources, not the statute text itself this session.
- Filing via **Virk.dk** (digital). Small class-B companies below size thresholds are exempt from mandatory audit (review/audit thresholds exist — not detailed this session).
- The standardkontoplan now includes all skema lines from **årsregnskabsloven Bilag 2, skema 1 & 3** (balance & P&L schedules) — i.e., the official chart is annual-report-schedule-aligned, which is the layout the profile's annual-report milestone should target.
- For bukio: annual report = **B-milestone** (regime note only; full Danish skema layout is out of scope for the bookkeeping CLI).

Sources: https://uniqorm.com/annual-reporting-in-denmark-key-legal-obligations-for-companies · https://gsl.org/en/audit-foreign/audit-denmark/ · https://proaktif.dk/denmark-annual-company-accounts · https://erhvervsstyrelsen.dk/standardkontoplan-saf-t
Confidence: **HIGH** (5 months, consistent across sources).

---

## 8. Fiscal year end

- **Default FYE = 31 December** (calendar year). Non-calendar FYE allowed (e.g. 1 Jul–30 Jun); the 2025/2026 standardkontoplan update takes effect at the start of the next accounting period accordingly.
Source: https://erhvervsstyrelsen.dk/standardkontoplan-saf-t (explicitly references both calendar and shifted FYE). Confidence: **HIGH**.

---

## 9. Banking

- **SEPA**: DKK IBAN = **DK + 2 check digits + 14 digits**, where the 14 digits = **4-digit bank code (reg.nr.) + 10-digit account number**. Domestic transfers use reg.nr. + account number (the last 10 digits of the IBAN). BIC/SWIFT used for international (e.g. DABADKKK for SKAT).
- **CAMT.053**: statement format is standard across Danish banks (Nordic banking norm), but **availability per bank was NOT verified this session** — flag. CAMT.053-debit-credit/end-of-day + CSV export are the common SME statement formats.
- Payment of taxes/VAT goes via the company **skattekonto** (tax account); refunds to **NemKonto** (mandatory public-payout account).

Sources: https://expatfinance.dk/banking/banks-in-denmark/ (reg.nr. 4 digits + account number) · https://www.scribd.com/document/761472541/Saxo-Standard-Settlement-Instructions (reg.nr. 1149; account = last 10 digits of IBAN) · https://marosavat.com/vat-manual-chapters/denmark-vat-returns (NemKonto, skattekonto)
Confidence: **HIGH** (numbering), **MED** (CAMT.053 — unverified).

---

## 10. Compliance calendar (SME)

| When | Obligation | Deadline |
|---|---|---|
| Monthly | VAT (only > DKK 50M turnover or on request) | 25th of following month |
| Quarterly | VAT (5–50M / newly registered) | 1st of 3rd following month |
| Half-yearly | VAT (< 5M, good history) | 1 Sep (H1) / 1 Mar (H2) |
| Monthly | A-skat & AM-bidrag (employer withholding) paid via skattekonto | ~10th of following month (flag: not verified this session) |
| 5 months after FYE | Annual report (årsrapport) filed at Erhvervsstyrelsen via Virk | class B small |
| Quarterly installments | Corporate income tax (selskabsskat 22%) acontobetaling via skattekonto | ~20 Mar / 1 Jun / 1 Sep / 1 Nov (flag: not verified this session) |
| **1 Jan 2026** | **Digital bogføringspligt** (digital bookkeeping obligation) takes effect for covered businesses; updated standardkontoplan mandatory from next period start | official |
| **1 Jan 2027** | Registered digital bookkeeping systems must support **SAF-T v2.0** (v1.0 header for non-registered) | official |

Sources: https://skat.dk/en-us/businesses/vat/deadlines-filing-vat-returns-and-paying-vat · https://erhvervsstyrelsen.dk/standardkontoplan-saf-t (extracted, official — the two 2026/2027 dates are verbatim)
Confidence: **HIGH** (VAT + official Erhvervsstyrelsen dates); payroll-tax and acontoskat dates **LOW/unverified**.

---

## 11. e-Invoicing

- **B2G: mandatory** — invoices to Danish public-sector buyers must be e-invoices (since ~2005 via **NemHandel**); historically **OIOUBL** format (UBL-based), which is what the public recipients require.
- **Transition in flux**: EU 2025 country sheet states **OIOUBL 3.0** was due to become mandatory **15 Nov 2025**; a vendor guide claims OIOUBL 3.0 was instead **cancelled** in favour of a new **NemHandel BIS 4** standard with phased rollout from **2028**. Flag: exact current status unverified — check NemHandel/DIGST before shipping.
- **B2B: voluntary** — Peppol network (Peppol BIS) used voluntarily; **Peppol BIS 3.0 UBL is the standard accepted for voluntary B2B** (Peppol BIS 3.0 is UBL 2.1-based) — **MED confidence**, not verified against a Danish-specific acceptance statement this session.
- **Peppol scheme ID: 0184** (DK:CVR, DIGSTORG) — active; identifier = 8-digit CVR. (Legacy 9902 DK:CVR also exists.)
- Suppliers send via registered **NemHandelsregisteret** (NemHandel register) services.

Sources: https://ec.europa.eu/digital-building-blocks/sites/pages/viewpage.action?pageId=905217510 (OIOUBL 3.0 → 15 Nov 2025) · https://dddinvoices.com/learn/e-invoicing-denmark (OIOUBL cancelled → NemHandel BIS 4, 2028) · https://marosavat.com/vat-news/e-invoicing-denmark-complete-guide (B2G mandatory since 2005; B2B voluntary) · https://peppolgate.eu/peppol-country-codes/ (0184 active) · https://learn.microsoft.com/en-us/dynamics365/business-central/localfunctionality/denmark/how-to-nemhandel-register
Confidence: **HIGH** (B2G mandatory, B2B voluntary, scheme 0184); **MED** (Peppol BIS 3.0 acceptance detail); **LOW/UNVERIFIED** (current OIOUBL vs NemHandel BIS 4 status).

---

## 12. Closing accounts (year-end)

- Convention: at year-end close **3990 Årets resultat** (result for the year) into **3120 Overført resultat** (retained earnings); the annual report equity section presents Overført overskud + Årets resultat separately.
- For sole traders (Enkeltmandsvirksomhed), the result is instead transferred to the owner's current account (mellemregning, e.g. 2380) — owner's drawings/private use post there; the result account and mellemregning effectively merge.
- Confidence: **MED** — consistent with the parent's draft codes and standard Danish practice; exact 3120/3990 codes are part of the unverified draft chart (see §5).

---

## 13. Currency / locale / formatting

- Currency: **DKK** (ISO 4217; krone, kr / øre). Locale: **da-DK**. Date format: **dd-mm-yyyy** (day first). Decimal separator comma, thousands dot — Danish convention.
- Confidence: **HIGH** (general knowledge; no source needed).

---

## Flags / unverifiable items (summary)

1. **"Monthly VAT mandatory since 2022 for all" is WRONG** — official SKAT table shows half-yearly/quarterly/monthly tiers by turnover; monthly only > DKK 50M or on request (deadline 25th, not the 1st). Quarterly = 1st of 3rd month; half-yearly = 1 Sep/1 Mar.
2. **Exact chart-of-accounts codes unverified** — e-conomic/Dinero publish charts as images/login-gated CSV; the 1xxx=assets scheme matches the official Erhvervsstyrelsen Standardkontoplan (JSON on git.erst.dk) rather than Dinero (1xxx=income) or e-conomic (13xx=costs). Verify against the official Standardkontoplan JSON before shipping.
3. **Exact VAT control account codes (2710/2720/2730) unverified**; naming (Salgsmoms/Købsmoms/Afregning af moms) is convention-safe.
4. **e-Invoicing status in flux** — OIOUBL 3.0 (15 Nov 2025) vs cancelled → NemHandel BIS 4 (2028); Peppol BIS 3.0 for voluntary B2B accepted (MED).
5. CAMT.053 availability, A-skat monthly deadline (~10th), acontoskat installments, ApS min. capital (40k vs 20k claim) — not verified this session.
6. skat.dk VAT-rates page returned http_error; rates confirmed via Avalara/Invoxo/Taxology instead.
