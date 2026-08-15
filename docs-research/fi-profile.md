# Finland (FI) — SME Bookkeeping Profile Brief

Research date: 2026-08-15. Currency EUR, locale fi-FI, date format dd.mm.yyyy.
Purpose: input for the bukio-cli `fi.js` jurisdiction profile. Confidence: HIGH / MEDIUM / LOW per item. Unverified items flagged.

---

## 1. VAT rates (2026) — HIGH

| Rate | Scope | Notes |
|---|---|---|
| **25.5%** general | most goods & services | raised from 24% on 1 Sep 2024 |
| **13.5%** reduced | groceries/food, restaurant & meal services, books (print+electronic), pharmaceuticals, sanitary products/diapers, sports & fitness services, cultural/entertainment event tickets, passenger transport, accommodation (hotels etc.), artist fees | was 14% until 31 Dec 2025; was 10% for books/meds/etc. until 1 Jan 2025 (widened to 14% from 2025, then 13.5% from 2026). NOT applicable to alcohol/tobacco. Public broadcasting services moved to 13.5% from 1 Jan 2026. |
| **10%** reduced | newspapers & magazines (print and electronic) only | |
| **0%** | exports outside EU, intra-Community sales to VAT-liable buyers, vessels, international transport | input VAT deductible |

Source: Vero (Finnish Tax Administration) "Rates of VAT" — https://www.vero.fi/en/businesses-and-corporations/taxes-and-charges/vat/rates-of-vat/ (page last updated 1/1/2026; confirms 1 Jan 2026: reduced rate 14% → 13.5%; 1 Jan 2025 widening; 1 Sep 2024: 24% → 25.5%). Corroborated: https://www.vatupdate.com/2026/02/04/finland-comprehensive-vat-country-guide-2026/ , https://fintua.com/country-vat-guides/europe-vat-guides/finland-vat-guide/

**Correction vs task brief:** the task assumed "14% food, 10% books/meds/accommodation" — that was the 2024 scheme. For 2026: food/restaurants/books/meds/accommodation = **13.5%**; 10% = newspapers/magazines only.

### Registration threshold + small-business exemption — HIGH
- Threshold **€20,000** annual turnover, in force from **1 Jan 2025** (raised from €15,000). Value for 2026: still €20,000.
- Small-business exemption (arvonlisäveroton vähäinen toiminta, formerly arvonlisäverovelvollisuuden alaraja): a seller is **not liable to VAT** if turnover of the current AND previous calendar year each ≤ €20,000 and not voluntarily registered. Assessment is by **calendar year** (not accounting year), over two consecutive years. Liability starts on the date the threshold is exceeded mid-year. Voluntary registration possible below the threshold. The old "alv-alarajahuojennus" (small-business VAT relief/refund) was **abolished from 1 Jan 2025**. Exempt sellers cannot deduct input VAT. Non-resident sellers: no threshold — must register immediately.
- Sources: Vero detailed guidance "VAT exemption for small businesses" (valid 1.1.2025→) — https://www.vero.fi/en/detailed-guidance/guidance/48658/vat-exemption-for-small-businesses/ ; Yle: https://yle.fi/a/74-20095124 ; https://www.vero.fi/en/businesses-and-corporations/taxes-and-charges/vat/vat-for-small-business/

## 2. VAT return (arvonlisäveroilmoitus / kausiveroilmoitus) — HIGH (frequency), MEDIUM (annual variant)

- Filing frequency by annual turnover (2026):
  - **Monthly** if turnover > €100,000
  - **Quarterly** if €30,000–€100,000
  - **Annual** if < €30,000
  - (Task brief said "monthly > €100k, quarterly otherwise" — **partially wrong**: sub-€30k businesses can file annually, due by end of February of the following year.)
- Deadline: **12th of the second month after the end of the tax period** (e.g. March return due 12 May; Q4 Oct–Dec due 12 Feb). VAT payment due same date.
- Filed electronically via OmaVero/MyTax; zero returns still required for every period.
- Return content (kausiveroilmoitus box numbers, per Netvisor docs): 301–303 VAT on domestic sales by rate, 305/306 VAT on EU goods/service purchases (reverse charge), 307 deductible VAT, 309 zero-rate turnover, 311/313/314 EU sales/purchases, 304/310 imports, 318–320 construction reverse charge.
- Cash-basis (maksuperusteinen) VAT available to companies with calendar-year turnover ≤ €500,000 (choice of accrual / invoice / cash basis, applied consistently; from 1 Jan 2017).
- Sources: https://marosavat.com/vat-manual-chapters/finland-vat-returns ; https://www.avalara.com/us/en/vatlive/country-guides/europe/finland/finnish-vat-returns.html ; https://1office.co/blog/vat-registration-finland-2026-guide/ (frequencies + deadlines; FAQ line "12th of the month following" contradicts its own detailed schedule — the 12th-of-2nd-month rule is corroborated by Marosa/Avalara); Vero cash-basis: https://www.vero.fi/yritykset-ja-yhteisot/verot-ja-maksut/arvonlisaverotus/pienyrityksen-maksuperusteinen-alv/ ; Netvisor VAT codes (box numbers): https://support.netvisor.fi/fi/articles/766539-arvonlisaveron-kasittely-ja-jarjestelman-alv-tunnisteet

## 3. Identifiers — HIGH

- **Y-tunnus** (Business ID / Business Identity Code): 7 digits + dash + check digit, e.g. `1234567-8`. Issued by PRH / Tax Administration; registry lookup at ytj.fi.
  - Source: https://www.ytj.fi/en/index/businessid.html ; https://hetut.fi/en/business-id-guide/
- **VAT number**: `FI` + 8 digits = Y-tunnus with dash removed (e.g. Y-tunnus 0112038-9 → FI01120389). 10 chars total.
  - Source: https://hetut.fi/en/business-id-guide/ ; https://www.commenda.io/blog/finland-vat-number-verification
- **Peppol participant scheme ID: `0037`** (Finnish tax administration organisation code). Participant identifier = `0037:FI<8-digit Y-tunnus>` (LY-tunnus = VAT number). OVT-code for Finnish e-invoicing is the same 0037 prefix + Y-tunnus without hyphen.
  - Sources: Peppol code lists — https://docs.peppol.eu/edelivery/codelists/old/v8.5/Peppol%20Code%20Lists%20-%20Participant%20identifier%20schemes%20v8.5.html (0037 "FI LY-tunnus National Board of Taxes"); https://documentation.maventa.com/services-and-reach/finland/ ; https://finbite.eu/en/e-invoice-to-finland/
  - Note: exact string length conventions differ slightly across docs ("8 characters with initial zero" vs FI+8digits); the practical Peppol ID is `0037:FI12345678`. Confidence on scheme 0037 HIGH; on exact ID construction MEDIUM.
- **accountNumber kind**: `iban` (FI + 2-digit BBAN; SEPA). Standard; no FI-specific source fetched — LOW (assumed from SEPA membership).

## 4. Legal forms — HIGH

- **Oy / osakeyhtiö** — private limited company, the SME standard (min. share capital €2,500, no min. since 2019 — actually min. share capital abolished 1 Sep 2019; not verified here).
- **Toiminimi** — sole trader (private trader; usually need NOT file financial statements with Trade Register).
- **Ky / kommandiittiyhtiö** — limited partnership; **Ay / avoin yhtiö** — general partnership.
- **Oyj / julkinen osakeyhtiö** — public limited company (listed).
- Sources: PRH limited liability companies — https://www.prh.fi/en/companiesandorganisations/yrityksen_perustaminen/osakeyhtio.html ; PRH financial statements page (private traders exempt) — https://www.prh.fi/en/companiesandorganisations/financial_statements.html ; https://kyckr.com/guides-and-reports/finland-business-registry

## 5. Chart of accounts (default SME chart) — HIGH (codes/labels from a current Finnish software model chart)

Finland has **no statutory chart of accounts**; software conventions dominate. The dominant numbering: 1xxx assets, 2xxx equity+liabilities, 3xxx revenue, 4xxx–8xxx costs. The reference below is the **Asteri/Liikekirjuri 2026 model chart** (`lk26.pdf`, widely used Liikekirjuri/Fivaldi-family SME chart, updated with the 2025/2026 VAT-rate accounts) — https://asteri.fi/tiedostot/tilikartat/lk26.pdf . Procountor also publishes a default chart (company version) — https://help.procountor.fi/en/articles/532542-chart-of-accounts (Excel attachments). Netvisor uses net-accounting with VAT codes instead of rate-split accounts — https://support.netvisor.fi/fi/articles/766539-arvonlisaveron-kasittely-ja-jarjestelman-alv-tunnisteet

Proposed ~40-account default chart (codes/labels as per Liikekirjuri 2026; NB = normal balance D=debit, C=credit):

| Code | Finnish label | EN label | Type | NB |
|---|---|---|---|---|
| 1021 | Kehittämismenot | Development expenditure | asset | D |
| 1041 | ATK-ohjelmat | Software/intangible rights | asset | D |
| 1051 | Liikearvo | Goodwill | asset | D |
| 1121 | Rakennukset | Buildings | asset | D |
| 1161 | Koneet ja laitteet | Machinery & equipment | asset | D |
| 1440 | Osakkeet ja osuudet | Shares and holdings | asset | D |
| 1501 | Aineet ja tarvikkeet | Materials and supplies (inventory) | asset | D |
| 1521 | Valmiit tuotteet | Finished goods (inventory) | asset | D |
| 1701 | Myyntisaamiset | Trade receivables | asset | D |
| 1761 | Verosaamiset | Tax receivables | asset | D |
| 1762 | Verotilisaamiset | Tax-account (verotili) receivables | asset | D |
| 1763 | Arvonlisäverosaamiset | VAT receivables (input VAT) | asset | D |
| 1800 | Siirtosaamiset | Accrued income / prepayments | asset | D |
| 1900 | Kassa | Cash | asset | D |
| 1910 | Pankkitili (Nordea) | Bank account | asset | D |
| 1970 | Pankkitili | Bank account (generic) | asset | D |
| 1990 | Pankkitilien väliset siirrot | Bank transfers | asset | D |
| 2001 | Osakepääoma | Share capital | equity | C |
| 2061 | SVOP-rahasto | Invested unrestricted equity fund | equity | C |
| 2201 | Peruspääoma | Sole-trader capital | equity | C |
| 2251 | Ed. tilikausien voitto/tappio | Retained earnings | equity | C |
| 2350 | Yksityistili | Sole-trader drawings | equity | D |
| 2375* | Tilikauden voitto (tappio) | Profit/loss for the year | equity | C |
| 2381 | Pääomalaina | Subordinated loan (equity-like) | equity | C |
| 2621 | Lainat rahoituslaitoksilta | Loans from financial institutions (LT) | liability | C |
| 2821 | Lainat rahoituslaitoksilta, lyhytaik. | Loans from financial institutions (ST) | liability | C |
| 2864 | Saadut ennakot | Advances received | liability | C |
| 2871 | Ostovelat | Trade payables | liability | C |
| 2939 | AV Verovelka | VAT liability (output VAT) | liability | C |
| 2963 | TyEL-velka | Pension (TyEL) liability | liability | C |
| 2979 | Siirtovelat | Accrued liabilities | liability | C |
| 3000 | Myynti ALV 25,5% | Sales 25.5% VAT | revenue | C |
| 3001 | Myynti ALV 13,5% | Sales 13.5% VAT | revenue | C |
| 3002 | Myynti ALV 10% | Sales 10% VAT | revenue | C |
| 3003 | Myynti 0% | Sales 0% VAT | revenue | C |
| 3004 | Myynti | Sales (no VAT code) | revenue | C |
| 3454 | Vuokratuotot | Rental income | revenue | C |
| 3994 | Liiketoiminnan muut tuotot | Other operating income | revenue | C |
| 4004 | Ostot | Purchases | expense | D |
| 4000 | Ostot ALV 25,5% | Purchases 25.5% VAT | expense | D |
| 4454 | Ulkopuoliset palvelut | External services | expense | D |
| 5000 | Palkat ja palkkiot | Salaries and fees | expense | D |
| 6100 | YEL-maksut | YEL pension insurance (self-employed) | expense | D |
| 6130 | TyEL-maksut | TyEL pension insurance (employer) | expense | D |
| 6300 | Sosiaaliturvamaksut | Social security contributions | expense | D |
| 6870 | Poisto koneista ja kalustosta | Depreciation of machinery & equipment | expense | D |
| 7214 | Vuokrat | Rent | expense | D |
| 7394 | Sähkö | Electricity | expense | D |
| 7864 | Matka- ja majoituskulut | Travel and accommodation | expense | D |
| 8054 | Mainoskulut | Marketing/advertising | expense | D |
| 8384 | Taloushallintopalvelut | Accounting/back-office services | expense | D |
| 8504 | Puhelin- ja tietoliikenne | Phone & telecom | expense | D |
| 8564 | Rahaliikenteen kulut | Bank charges | expense | D |
| 8624 | Toimistokulut | Office expenses | expense | D |
| 8704 | Luottotappiot | Bad debts | expense | D |
| 8764 | Muut kulut | Other expenses | expense | D |
| 8804 | Vähennyskelvottomat kulut | Non-deductible expenses | expense | D |
| 9460 | Korkokulut lainoista | Interest on loans | expense | D |
| 9900 | Ennakkoverot | Prepaid income tax | expense | D |

*2375: exact result-account code not printed in the PDF (heading only); Liikekirjuri variant list "(2251,2090,2371,2375)" covers retained/result accounts across chart versions. Task-guess codes that do NOT match this convention: 2970/2971/2974 VAT (Liikekirjuri uses 2939/1763), 2080 share capital (→2001), 4100 wages (→5000), 4700 rent (→7214), 5800 office (→8624), 2320 payroll (→2963 for TyEL etc.). Some other charts (e.g. Procountor default) do use the 29xx/2xxx differently — see §6.

## 6. VAT control accounts (convention) — HIGH (Liikekirjuri), MEDIUM (cross-software)

Dominant Finnish balance-sheet convention (Liikekirjuri 2026, and similar in Fivaldi-family charts):
- **Output VAT** (myyntien ALV) → liability, sub-accounts of **2939 "AV Verovelka"**: `29390 Myynnin 25,5% alv-velka`, `29391 Myynnin 13,5% alv-velka`, `29392 Myynnin 10% alv-velka`, plus 29395/29396/29397/29398 for EU/reverse-charge VAT liabilities.
- **Input VAT** (ostojen ALV) → asset, sub-accounts of **1763 "Arvonlisäverosaamiset"**: `17630 Ostojen 25,5% ALV-saaminen`, `17631 Ostojen 13,5% ALV-saaminen`, `17632 Ostojen 10% ALV-saaminen`, plus EU/import/construction variants.
- **Settlement**: net (2939x − 1763x) is paid to/refunded from the **verotili** (tax account); balance-sheet netting to 2939 or 1763. Liikekirjuri chart also carries `1762 Verotilisaamiset`.
- Netvisor: VAT posted automatically to system "ALV-velkatili" / "ALV-saamistili" via VAT codes (KOMY domestic sales → VAT liability account; KOOS domestic purchase → VAT receivable account) — account numbers are configurable, not fixed.
- Task-brief guess 2970/2971/2974 does appear in SOME charts (e.g. a Finnish thesis shows `2977 Alv-siirtovelka`); 297x is used for accruals (2979 Siirtovelat) in Liikekirjuri. **Recommendation: use 2939/1763 as the primary convention for fi.js.**
- Sources: https://asteri.fi/tiedostot/tilikartat/lk26.pdf ; https://support.netvisor.fi/fi/articles/766539-arvonlisaveron-kasittely-ja-jarjestelman-alv-tunnisteet ; https://www.theseus.fi/bitstream/10024/340290/2/Uittoluoto_Taija_Lehikoinen_Anne.pdf (2977 Alv-siirtovelka example)

## 7. Statutory accounts (tilinpäätös) — HIGH

- Regime: Finnish Accounting Act (kirjanpitolaki 1336/1997) + Accounting Ordinance (kirjanpitoasetus 1339/1997); annual accounts = tilinpäätös (balance sheet, income statement, notes, optional cash flow). Public layout formats (tuloslaskelma/tase) prescribed by the ordinance — https://www.finlex.fi/fi/lainsaadanto/1997/1339
- **Small companies (incl. Oy) must prepare and present financial statements within 4 months of the financial year end** (Companies Act; confirm: yes for osakeyhtiö — sources below). Large companies 6 months.
- Filing with the **PRH Trade Register**: after adoption by the general meeting; free of charge if filed online at ytj.fi. Absolute deadline **8 months from end of financial period** — late fee applies after 8 months (escalating; after ~1 year risk of deregistration/liquidation). (Task brief said "filed 1 month after adoption" — **not confirmed**; the operative rules are: within 2 months of adoption per Companies Act, and hard stop 8 months from FYE per PRH.)
- **XBRL/iXBRL structured filing becomes mandatory from 2027** for most companies (replacing PDF filing).
- Sources: PRH "Filing financial statements" (timeline) — https://www.prh.fi/en/companiesandorganisations/financial_statements.html ; https://1office.co/finland/services/annual-reports/ (4 months); https://www.azets.com/en-fi/resources/blog/new-finnish-trade-register-act (8 months/late fee); https://www.xbrl.org/news/finland-moves-to-mandatory-xbrl-reporting-for-company-accounts/

## 8. Fiscal year end — HIGH (convention)

Default calendar year **31.12** (12 months); any other 12-month period allowed (kirjanpitolaki). Finnish software charts are calendar-year oriented (Liikekirjuri sample: 1.1.–31.12.2026).

## 9. Banking — MEDIUM (partly standard knowledge)

- SEPA member; IBAN format `FI` + 18 digits (FI + BBAN). BIC/SWIFT required for cross-border.
- Bank statement formats: CAMT.053 (ISO 20022) is the standard machine-readable statement used by Finnish banks/APIs; netvisor/procountor-class tools import CAMT.053. Not independently verified with a fetched source in this run — flag as assumed standard.

## 10. Compliance calendar (SME) — HIGH

- **VAT (kausiveroilmoitus)**: monthly / quarterly / annual per §2; due **12th of the second month after the period**; annual return due end of February. Filed via OmaVero.
- **Tax account (verotili)**: VAT, withholding, employer contributions settle via the tax account; payment date = return due date (12th of 2nd month).
- **Employer**: monthly withholding & social contributions via verotili + **Incomes Register (tulorekisteri)** reporting within 5 days of payment (not deeply verified here — MEDIUM).
- **Corporate income tax**: 20% (2025/2026); advance payments (ennakkoverot) monthly, tax return (veroilmoitus) for companies due ~4 months after FYE (not verified in this run).
- **Annual accounts**: prepared ≤4 months after FYE; filed with PRH ≤8 months after FYE (free online).
- Late VAT filing penalties up to €15,000 + 7% interest (per 1Office 2026 guide — MEDIUM, secondary source).
- Sources: as in §2/§7; https://1office.co/blog/vat-registration-finland-2026-guide/

## 11. e-Invoicing — HIGH

- **B2B e-invoicing is NOT mandatory in Finland.** The task brief's assumption of a 2025-04-01 B2B mandate is **FALSE** (EU 2025 country sheet: B2B mandate NO, B2C mandate NO; only **B2G is mandatory** — public bodies accept only EN-compliant e-invoices since April 2020/2021).
- However, under the Finnish e-Invoicing Act (241/2019), **B2B companies with annual turnover > €10,000 are entitled to request e-invoices from their suppliers** (supplier must send e-invoice on request).
- Standards: EN 16931 implemented (UBL 2.1 / CII); national formats **Finvoice 3.0** and TEAPPSXML 3.0 accepted when EN-compliant; **Peppol BIS Billing 3.0 accepted** for transmission over the Peppol network. Peppol scheme ID 0037 (§3).
- Government preparing e-invoicing for intra-EU transactions by 2028.
- Sources: EU Commission 2025 eInvoicing Country Sheet (Finland) — https://ec.europa.eu/digital-building-blocks/sites/pages/viewpage.action?pageId=905217496 ; https://peppol.org/e-invoicing-without-borders-finland-and-germany/

## 12. Closing accounts — HIGH (convention), MEDIUM (exact codes)

- Convention: at year-end, P&L accounts close to the **result account "Tilikauden voitto (tappio)"** (year profit/loss; codes vary by chart — Liikekirjuri family 2371/2375, heading in §5 chart; not printed as an exact number in the source PDF). After the general meeting adopts the accounts, the result is transferred to **retained earnings "Edellisten tilikausien voitto/tappio"** — Liikekirjuri 2026: **2251** (variant list also names 2090, used by some other charts).
- Procountor closing flow: closing of accounts tools → transfer of profit/loss account value (https://help.procountor.fi/en/articles/531565-closing-of-accounts-tools).

## 13. Locale — HIGH

- Currency: **EUR**; locale: **fi-FI**; date format: **dd.mm.yyyy**; decimal comma, thousands space (Finnish number format, e.g. 1 234,56 €).

---

## UNVERIFIED / LOW-CONFIDENCE ITEMS
- Exact Peppol participant ID string length convention for FI (`0037:FI12345678` vs 8-char tax ID reading in the code list) — MEDIUM.
- CAMT.053 as the bank-statement standard for all Finnish banks — assumed, no source fetched.
- Account code for the result account (2371 vs 2375) and for "Tilikauden voitto" in the Liikekirjuri chart — the PDF shows the heading without an explicit number.
- Annual VAT filing for <€30k turnover and its end-of-February deadline — single (1Office) source; vero.fi not fetched for this.
- Incomes Register 5-day reporting and corporate income-tax return deadline — not verified with a primary source.
- Late-filing penalties (€15,000 / 7%) — secondary source only.
- Oy minimum share capital €2,500 (abolished 2019) — not verified.
- "Filed 1 month after adoption" (task brief) — not confirmed; actual rule is 2 months after adoption / 8 months from FYE.
