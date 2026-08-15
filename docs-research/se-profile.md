# Sweden (SE) — SME Bookkeeping Jurisdiction Profile

Research brief for the `se.js` jurisdiction profile of bukio-cli.
Compiled 2026-08-15 via web research. Confidence: **High** unless flagged. Items marked ⚠️ could not be fully verified.

---

## 1. VAT rates & registration (2026)

**Rates** (High — [Skatteverket, VAT rates and VAT exemption](https://www.skatteverket.se/servicelankar/otherlanguages/englishengelska/businessesandemployers/startingandrunningaswedishbusiness/vat/vatratesandvatexemption.4.676f4884175c97df419255d.html)):
- Standard **25 %** (most goods/services)
- Reduced **12 %**: hotel accommodation, restaurant/café meals, works of art, bicycle/shoe/clothing repairs, etc.
- Reduced **6 %**: books/newspapers (not directly in extract but standard), theatre/concert admission, sports, cultural events, passenger transport, copyright transfers
- **0 %/exempt**: exports, financial/insurance services, artist performance fees, business transfers
- ⚠️ **2026 change**: from **1 April 2026** food sales (incl. takeaway) drop from 12 % → **6 %**, while food/drinks consumed at a restaurant/café stay at **12 %** (Skatteverket page above). The end date (31 Dec 2027, temporary reduction) is from a secondary source: [Fiscal Solutions](https://www.fiscal-requirements.com/news/5131-change-of-vat-rates-from-april-2026-in-sweden) — flag as unverified against Skatteverket.

**Registration threshold — the task premise is WRONG.** Sweden DOES have a small-business VAT exemption (SME scheme, EU 2020/285): annual turnover **≤ SEK 120,000** → automatically exempt from VAT registration (must be based in Sweden, and turnover ≤ 120k in the two preceding years too). Ends the moment sales exceed 120k. ([Skatteverket — In certain cases you do not need to register for VAT](https://www.skatteverket.se/servicelankar/otherlanguages/englishengelska/businessesandemployers/startingandrunningaswedishbusiness/registeringabusiness/incertaincasesyoudonotneedtoregisteryourbusinessforvat.4.6e1dd38d196873bc1e1cff.html)). The **SEK 80,000** figure is outdated (interim value raised in 2025 — see [VATupdate 2026 guide](https://www.vatupdate.com/2026/02/02/sweden-comprehensive-vat-country-guide-2026/)); current official limit = **120,000**. No input VAT deduction while exempt.

## 2. VAT return (momsredovisning)

**Frequency** (High — [Skatteverket, När ska jag deklarera moms](https://www.skatteverket.se/foretag/moms/deklarera-moms/nar-ska-jag-deklarera-moms.4.6d02084411db6e252fe80008988.html)):
- Taxable base **> SEK 40 M** → **monthly** (no choice)
- ≤ 40 M → **quarterly** (can choose monthly)
- ≤ 1 M → **annually** possible (sole traders w/o EU trade: 12 May; w/ EU trade: 26 Feb; AB/ek. förening w/o EU trade: tied to income-tax declaration dates)

**Deadlines** (High, official page above; VAT must be **both declared AND paid** on the deadline day, via the skattekonto):
- **Quarterly**: 12th of the **second** month after the period (August: 17th). *Task assumption of "26th" is WRONG.*
- **Monthly**, base ≤ 40 M: 12th of the **second** month after the period (Jan & Aug: 17th)
- **Monthly**, base > 40 M: 26th of the **month after** the period (Dec: 27th)
- **Annual** (see frequency)
- Late filing fee: SEK 625 (1,250 if the agency demands it). Payment interest = base rate + 15 pp ([Marosa VAT manual](https://marosavat.com/vat-manual-chapters/sweden-vat-returns)).

**VAT return boxes** (High — [Skatteverket, Fill in the VAT return](https://www.skatteverket.se/servicelankar/otherlanguages/englishengelska/businessesandemployers/startingandrunningaswedishbusiness/declaringtaxesbusinesses/vatdeclarations/fillinthevatreturn.4.3dfca4f410f4fc63c8680004502.html)): output VAT in boxes 10 (25 %), 11 (12 %), 12 (6 %), plus 30/31/32 (reverse charge), 60/61/62 (import); input VAT in box 48; net in box 49.

## 3. Identifiers

- **Organisationsnummer** (company number): 10 digits, format `556677-8899` (6 digits + hyphen + 4 digits; last digit = check digit). High — [Commenda](https://www.commenda.io/blog/sweden-vat-number-verification), [EUIPO VAT numbers PDF](https://euipo.europa.eu/tunnel-web/secure/webdav/guest/document_library/Documents/COSME/VAT%20numbers%20EU.pdf).
- **VAT number**: `SE` + 10-digit org.nr + fixed suffix `01` = **12 digits**, e.g. `SE556677889901`. High (same sources).
- **Peppol scheme ID**: **0007** (Organisationsnummer — org.nr without hyphen) used for `endpointId` on Peppol BIS 3.0 invoices to Swedish buyers. Medium — [Clearvo SE dev guide](https://clearvo.io/countries/se); not cross-checked against the official Peppol registry in this pass. ⚠️
- **accountNumber kind**: `iban`. High.

## 4. Legal forms

High — [Verksamt.se — Välj företagsform](https://verksamt.se/starta-foretag/valj-foretagsform):
- **AB** — aktiebolag (limited company; the SME standard, min share capital SEK 25,000)
- **Enskild firma / enskild näringsverksamhet** — sole trader
- **HB** — handelsbolag (general partnership)
- **KB** — kommanditbolag (limited partnership)
- **Ekonomisk förening** — economic association (cooperative)

## 5. Chart of accounts — BAS-kontoplan (de-facto standard, NOT statutory)

No statutory chart in Sweden; BAS 2023 (BAS-kontoplan, from [bas.se](https://www.bas.se/wp-content/uploads/2022/12/Kontoplan-2023.pdf.)) is the de-facto standard used by all Swedish software. Verified against the full BAS 2023 printout ([Visma/Spirís hb_bas2023.pdf](https://support.spiris.se/visma-fakturering/content/resources/files/pdf/kontoplan/adm/hb_bas2023.pdf)) and the K2 (small-company) version ([eDeklarera BAS kontoplan](https://edeklarera.se/arsredovisning/kontoplan-bas)). Structure: 4-digit codes; class 1 = assets, 2 = equity/liabilities, 3 = revenue, 4 = material costs, 5–6 = other external costs, 7 = personnel/PP&E, 8 = financial items.

### Default chart (AB-oriented, ~45 accounts; codes verified against BAS 2023)

| Code | Swedish label (EN) | Type | Normal balance |
|---|---|---|---|
| 1110 | Byggnader (Buildings) *(optional)* | asset | debit |
| 1210 | Maskiner och andra tekniska anläggningar (Machinery) | asset | debit |
| 1220 | Inventarier och verktyg (Fixtures & tools) | asset | debit |
| 1240 | Bilar och andra transportmedel (Vehicles) *(optional)* | asset | debit |
| 1460 | Lager av handelsvaror (Inventory of goods) *(optional)* | asset | debit |
| 1510 | Kundfordringar (Accounts receivable) | asset | debit |
| 1630 | Avräkning för skatter och avgifter – skattekonto (Tax account) | asset | debit |
| 1650 | Momsfordran (VAT receivable) | asset | debit |
| 1910 | Kassa (Cash) | asset | debit |
| 1930 | Företagskonto/checkkonto/affärskonto (Bank/current account) | asset | debit |
| 2081 | Aktiekapital (Share capital) | equity | credit |
| 2091 | Balanserad vinst eller förlust (Retained earnings) | equity | credit |
| 2098 | Vinst eller förlust från föregående år (Prior-year result) | equity | credit |
| 2099 | Årets resultat (Profit/loss for the year) | equity | credit |
| 2420 | Förskott från kunder (Advances from customers) | liability | credit |
| 2440 | Leverantörsskulder (Accounts payable) | liability | credit |
| 2510 | Skatteskulder (Tax liabilities) | liability | credit |
| 2611 | Utgående moms på försäljning inom Sverige, 25 % (Output VAT 25 %) | liability | credit |
| 2621 | Utgående moms på försäljning inom Sverige, 12 % (Output VAT 12 %) | liability | credit |
| 2631 | Utgående moms på försäljning inom Sverige, 6 % (Output VAT 6 %) | liability | credit |
| 2641 | Debiterad ingående moms (Input VAT) | liability | debit |
| 2650 | Redovisningskonto för moms (VAT settlement) | liability | credit |
| 2710 | Personalskatt (Withholding tax on salaries) | liability | credit |
| 2730 | Lagstadgade sociala avgifter o särskild löneskatt (Statutory social contributions) | liability | credit |
| 2850 | Avräkning för skatter och avgifter – skattekonto (Tax account) | liability | credit |
| 2990 | Övriga upplupna kostnader o förutbetalda intäkter (Accruals) | liability | credit |
| 3001 | Försäljning inom Sverige, 25 % moms (Domestic sales 25 %) | revenue | credit |
| 3002 | Försäljning inom Sverige, 12 % moms | revenue | credit |
| 3003 | Försäljning inom Sverige, 6 % moms | revenue | credit |
| 3041 | Försäljning tjänster 25 % moms Sv (Domestic services 25 %) | revenue | credit |
| 3042 | Försäljning tjänster 12 % moms Sv | revenue | credit |
| 3043 | Försäljning tjänster 6 % moms Sv | revenue | credit |
| 4010 | Inköp av varor och material (Purchases of goods/materials) | expense | debit |
| 5010 | Lokalhyra (Rent of premises) | expense | debit |
| 5060 | Städning och renhållning (Cleaning) | expense | debit |
| 5410 | Förbrukningsinventarier (Consumables) | expense | debit |
| 6100 | Kontorsmateriel och trycksaker (Office supplies) | expense | debit |
| 6200 | Tele och post (Telephone & post) | expense | debit |
| 6310 | Företagsförsäkringar (Business insurance) | expense | debit |
| 6540 | IT-tjänster (IT services) | expense | debit |
| 6570 | Bankkostnader (Bank charges) | expense | debit |
| 6590 | Övriga externa tjänster (Other external services) | expense | debit |
| 6990 | Övriga externa kostnader (Other external costs) | expense | debit |
| 7010 | Löner till kollektivanställda (Wages, manual workers) | expense | debit |
| 7210 | Löner till tjänstemän (Salaries, staff) | expense | debit |
| 7511 | Lagstadgade sociala avgifter för löner och ersättningar (Statutory social security costs) ⚠️ label | expense | debit |
| 8400 | Räntekostnader (Interest costs) | expense | debit |
| 8999 | Årets resultat (Result for the year — P&L summary) | p&l | credit/debit |

**Important corrections to the task's guesses** (all verified in BAS 2023 sources above):
- Bank: **1930** is the bank account (Företagskonto); **1910 = Kassa** (cash), not bank.
- Buildings = **1110** (not 1120); 1120 = Förbättringsutgifter på annans fastighet. Machinery = **1210**, inventory/fixtures = **1220**, vehicles = **1240**, computers = 1250.
- Equity: 2081 Aktiekapital ✓, **2098** Vinst eller förlust från föregående år ✓ (also 2091 Balanserad vinst eller förlust in the K2 chart), **2099** Årets resultat ✓. 2080 = Bundet eget kapital, 2090 = Fritt eget kapital.
- Revenue: 3001-3004 by rate (25/12/6/momsfri) for goods; **3041/3042/3043** by rate for services (NOT "3041 = services 25 %" as a single generic — it exists but is rate-specific).
- Costs: **5010 = Lokalhyra** (NOT Löner); **5060 = Städning och renhållning** (NOT sociala avgifter); **6310 = Företagsförsäkringar** (NOT Hyror); **6570 = Bankkostnader** (NOT räntor); **6200 = Tele och post** (6100 = Kontorsmateriel); **6990 = Övriga externa kostnader** ✓. Wages are class 7: **7010/7210**; social security costs: **7511**; interest: **8400** (8410 long-term, 8420 short-term).

## 6. VAT control accounts (BAS 2023)

High — [hb_bas2023.pdf](https://support.spiris.se/visma-fakturering/content/resources/files/pdf/kontoplan/adm/hb_bas2023.pdf), [Björn Lundén (2611→ruta 10)](https://support.bjornlunden.se/guide/skattedeklarationen-ruta-for-ruta):
- **Output VAT**: 2611 (25 %, box 10), **2621** (12 %, box 11), **2631** (6 %, box 12) — for domestic sales. Sub-accounts: 2612/2622/2632 egna uttag, 2613/2623/2633 uthyrning, 2614/2624/2634 omvänd skattskyldighet (boxes 30/31/32), 2615/2625/2635 import (boxes 60/61/62), 2616/2626/2636 VMB. *Task guess "2611/2612/2613 by rate" is NOT the BAS 2023 numbering* (older charts used it — version-dependent ⚠️).
- **Input VAT**: **2641 Debiterad ingående moms** (single account, ALL rates, box 48). 2642 = frivillig skattskyldighet, 2645 = beräknad ingående moms EU acquisitions, 2647 = omvänd skattskyldighet in Sweden. *No 2642/2643 by rate in BAS 2023.*
- **Settlement**: **2650 Redovisningskonto för moms** (box 49). VAT receivable asset side: 1650 Momsfordran.
- Skattekonto: 1630 (asset) / 2850 (liability) Avräkning för skatter och avgifter.

## 7. Statutory accounts (årsredovisning)

High — [Bolagsverket — Årsredovisning för aktiebolag](https://bolagsverket.se/foretag/aktiebolag/arsredovisningforaktiebolag.759.html), [Bolagsverket — ta fram en årsredovisning](https://bolagsverket.se/foretag/aktiebolag/arsredovisningforaktiebolag/taframenarsredovisning.793.html), [Verksamt](https://verksamt.se/bokforing/arsbokslut-arsredovisning/aktiebolag):
- Annual report must be **prepared within 6 months** of FYE (Bokföringslagen 8:3) — late = possible bokföringsbrott ([co-redovisning](https://www.co-redovisning.se/blogg/datumarsredovisning)).
- Adopted at the AGM (within 6 months), then **filed with Bolagsverket within 1 month of adoption**; hard cap **7 months after FYE**.
- Small companies use **K2** (BFNAR 2016:10) or K3 (BFNAR 2012:1). Regime note only — layout is a B-milestone in bukio-cli.
- Sole traders (enskild firma) generally do NOT file annual reports (only bokslut + income tax return).

## 8. Fiscal year end

Default **31 December** (calendar year). High — standard, cf. [Skatteverket](https://www.skatteverket.se/foretag/inkomstdeklaration/deklareraatettaktiebolagellerenekonomiskforening.4.46ae6b26141980f1e2d1261.html) ("ett aktiebolag har ett räkenskapsår som slutar 31 december").

## 9. Banking

- SEPA member; **IBAN = 24 chars** (`SEkk BBB CCCC CCCC CCCC CCCC`, starts SE) — High, [Wise](https://wise.com/us/iban/sweden).
- Domestic payments: **clearing number (4–5 digits) + account number** (≤ 10 digits); IBAN is not required domestically — Medium/High, [Wamo help](https://help.wamo.io/en/articles/16131071-sweden-your-account-number-clearing-number-and-how-payments-work). Note: IBAN bank-code segment length differs by source (3 vs 4 digits) ⚠️.
- Bank statement format: **CAMT.053 (ISO 20022)** supported — High for Handelsbanken ([official](https://www.handelsbanken.com/en/our-services/digital-services/global-gateway/iso-20022-xml), camt.053 v2); Swedish banks migrated to ISO 20022 NPC (Nordea news). "All Swedish banks" claim ⚠️ not exhaustively verified.

## 10. Compliance calendar (SME, AB, calendar year)

| When | What | Ref |
|---|---|---|
| Monthly, 12th (Jan/Aug: 17th) | Employer declaration (arbetsgivardeklaration) + pay withholding tax/social contributions | [Skatteverket](https://www.skatteverket.se/foretag/arbetsgivare/lamnaarbetsgivardeklaration/narskajaglamnaarbetsgivardeklaration.4.361dc8c15312eff6fd13c11.html), [Verksamt](https://verksamt.se/personal-rekrytering/vad-kostar-det/redovisa-och-betala-arbetsgivaravgifter) |
| Quarterly (≤40 M), 12th of 2nd month (Aug: 17th) | Momsredovisning + payment | [Skatteverket](https://www.skatteverket.se/foretag/moms/deklarera-moms/nar-ska-jag-deklarera-moms.4.6d02084411db6e252fe80008988.html) |
| Monthly (>40 M), 26th of following month (Dec: 27th) | Momsredovisning + payment | same |
| 1 August (next working day) | Income tax return (Inkomstdeklaration 2/3) for AB with calendar FYE | [Skatteverket](https://www.skatteverket.se/foretag/inkomstdeklaration/deklareraatettaktiebolagellerenekonomiskforening.4.46ae6b26141980f1e2d1261.html) |
| ≤ 6 months after FYE | Annual report prepared + adopted at AGM | Bolagsverket (§7) |
| ≤ 7 months after FYE (1 month after adoption) | Årsredovisning filed with Bolagsverket | Bolagsverket (§7) |

## 11. e-Invoicing

High — [European Commission — eInvoicing in Sweden](https://ec.europa.eu/digital-building-blocks/sites/pages/viewpage.action?pageId=758743138):
- **B2G: mandatory** — public-sector buyers must receive e-invoices via the **Peppol network**, format **Peppol BIS Billing 3.0** (EN 16931). Svefaktura is the legacy Swedish standard (superseded for public sector). Law ref (SFS 2018:1277, in force 1 Apr 2019) ⚠️ not directly extracted.
- **B2B: voluntary** — Peppol BIS 3.0 UBL accepted; widely used.
- **Peppol scheme ID 0007** (Organisationsnummer) for SE identifiers — Medium, [Clearvo](https://clearvo.io/countries/se); EC page confirms Peppol BIS 3.0 but not the scheme ID.

## 12. Closing accounts (BAS)

High — [FAR Online — Årets resultat](https://www.faronline.se/dokument/rattserien/redovisa-ratt/a2/rr_aretsresultat/), [Fortnox](https://www.fortnox.se/fortnox-foretagsguide/bokforingstips/resultat-i-aktiebolag), [Björn Lundén](https://support.bjornlunden.se/guide/arets-resultat):
- P&L result closes to **8999 Årets resultat** (group 8990 Resultat).
- Balance-sheet result account: **2099 Årets resultat**; after adoption, transfer 2099 → **2098 Vinst eller förlust från föregående år** at the start of the new year (2091 Balanserad vinst eller förlust also exists in K2 charts).

## 13. Currency / locale / format

- Currency: **SEK** (minor units 2). Locale: **sv-SE**. Date format: **yyyy-mm-dd** (ISO 8601, statutory in Swedish bookkeeping).
- High — standard conventions.

---

## ⚠️ Unverified / flagged items
1. **SEK 120,000 SME exemption** — current official (Skatteverket); the 80,000 figure seen in mid-2025 reporting is superseded. The premise "Sweden has no small-business exemption" is **false**.
2. **Food VAT 6 % end date (31 Dec 2027)** — only from Fiscal Solutions, not Skatteverket.
3. **Peppol scheme ID 0007** — from Clearvo dev guide; not cross-checked with official Peppol/EC registry.
4. **B2G mandate law reference (SFS 2018:1277)** — EC page confirms mandate + Peppol BIS 3.0; law number not extracted directly.
5. **7511 label wording** — "Lagstadgade sociala avgifter för löner och ersättningar" from a BAS-based chart (Svensk Handboll); the BAS 2023 printout slice showed the 2730 balance-sheet counterpart but not this exact line.
6. **Old VAT-code numbering (2612/2613 as 12 %/6 %)** — used by some legacy charts; BAS 2023 uses 2611/2621/2631. Version-dependent.
7. **CAMT.053 "all banks"** — confirmed for Handelsbanken; Nordic ISO 20022 migration reported by Nordea; not exhaustively verified per bank.
8. **IBAN BBAN structure** — 24 chars confirmed; internal segment lengths differ between sources (3 vs 4-digit clearing).
9. **Annual VAT filing for AB without EU trade** — dates tied to income-tax declaration; exact dates depend on FYE (table on the Skatteverket page).
