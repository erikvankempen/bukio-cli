# Belgium (BE) SME bookkeeping profile — research brief

Jurisdiction profile data for `src/jurisdictions/be.js`. Companion to the LU profile pattern (`lu.js`). Legal base: Belgian VAT Code (WBTW/Code TVA), Code of Companies and Associations (WVV/CSA, since 2019), Royal Decree of 12-09-1983 (minimum chart of accounts, PCMN/minimumrekeningenstelsel).

Confidence: **high** = official source or multiple independent sources; **medium** = single reputable source or reasonable inference; **low** = domain knowledge, unverified this run. Every claim carries a source URL.

---

## 1. VAT rates 2026 + small-business franchise scheme

| Item | Value | Confidence |
|---|---|---|
| Standard rate | **21%** | high |
| Reduced rate 1 | **12%** (restaurants/catering on-site excl. alcohol, social housing, hotel stays/takeaway meals from 1 Mar 2026, sport/leisure from 1 Mar 2026) | high |
| Reduced rate 2 | **6%** (foodstuffs, water, pharma, books, passenger transport, renovations >10 y, some energy) | high |
| Zero rate | **0%** exists for exceptional cases (exports, intra-EU supplies, some international services) — not a general category | high |
| Registration / franchise threshold | **€25,000** annual turnover (excl. VAT) → **VAT exemption scheme for small businesses** ("régime de la franchise de taxe" / "kleine ondernemingsregeling" / SME scheme) | high |

Sources:
- https://www.eurofiscalis.com/en/vat-rules-belgium/ (21/12/6/0)
- https://www.vatupdate.com/2026/01/10/comprehensive-vat-guide-belgium-2026/ (21/12/6 + 0% exceptional; rate changes 1 Mar 2026)
- https://www.vatcalc.com/belgium/belgium-hotel-takeaway-meal-vat-rises-1-march-2026/ (12% moves 1 Mar 2026)
- https://lawsupport.be/blog/belgium-vat-guide/

Franchise scheme details (https://finance.belgium.be/en/enterprises/vat/vat-obligation/vat-exemption-scheme-small-businesses — FPS Finance, official):
- **Effective 1 January 2025** (NOT 2024 — the task hypothesis is wrong). Belgian implementation of the EU SME scheme (directive (EU) 2020/285); legal base: art. 56bis–56undecies WBTW + Royal Decree No. 19 of 15-12-2024. The older domestic regime (≤ €15k/€25k without EU recognition) ended 31-12-2024.
- Threshold: annual turnover **≤ €25,000 excl. VAT**, pro-rated for start-ups mid-year; **10% tolerance** (≤ €27,500) before losing the scheme for the next calendar year; >10% over → normal regime from the transaction causing the excess.
- Excluded sectors: construction work, businesses required to use a certified cash register (POS), supply of old materials, VAT units.
- Consequences: no periodic VAT returns, no VAT charged, no VAT deduction; still needs a VAT ID, still files e604 A/B/C start/change/stop declarations, and submits the annual **client listing (customer list)** by **31 March** (first time 31 Mar 2026 for 2025; nihil list required if no turnover).
- Invoicing: exempt businesses do **not charge VAT**; invoices must mention the exemption. **UNVERIFIED this run:** the exact mandatory invoice wording (expected reference to art. 56bis WBTW, e.g. "vrijgesteld van BTW / exonéré de TVA — art. 56bis WBTW"). The FPS brochure link exists but was not extracted: https://www.minfin.fgov.be/myminfin-web/pages/public/fisconet/document/2b18be46-ea25-4430-93e0-3c8df0bf7fb9 — flag for B-milestone verification.

## 2. VAT return: frequency, deadline, annual listing

| Item | Value | Confidence |
|---|---|---|
| Default frequency | **Monthly** (all VAT-liable businesses above the small-business exemption) | high |
| Quarterly eligibility | Turnover in Belgium **≤ €2,500,000/yr** AND intra-EU supplies < €50,000 per quarter → may file quarterly (option, not automatic) | high |
| Monthly deadline | Return **and payment due the 20th** of the month following the period (via Intervat) | high |
| Quarterly deadline | **25th** of the month following the quarter | high |
| Annual listing | "jaarlijkse listing" / "liste annuelle des clients": B2B client listing filed via Intervat **by 31 March** of the following year; includes B2B sales **> €250 excl. VAT** | high |

Notes:
- The "monthly by default since 2022" framing: monthly has been the norm; since 2022 businesses with turnover > €2.5M are **required** monthly, smaller ones may keep quarterly. Late-filing penalty €100/month (max €500).
- Sources:
  - https://marosavat.com/vat-manual-chapters/belgium-vat-returns (monthly 20th, quarterly 25th, €2.5M condition)
  - https://meridianglobalservices.com/country-profile/belgium/ (monthly 20th, quarterly 25th)
  - https://www.avalara.com/us/en/vatlive/country-guides/europe/belgium/belgian-vat-returns.html (€2.5M + €50k/quarter condition; annual listing 31 March; €100/mo penalty)
  - https://marosavat.com/vat-news/annual-sales-listing-belgium (listing by 31 March)

## 3. Identifiers

| Item | Value | Confidence |
|---|---|---|
| Company/enterprise number | **KBO/BCE Ondernemingsnummer / Numéro d'entreprise** — 10 digits starting with **0 or 1**, displayed `0xxx.xxx.xxx`; last 2 digits = mod-97 checksum. Assigned by Crossroads Bank of Enterprises (KBO/BCE, FPS Economy) | high |
| VAT number | **BE + 10 digits** (`BE0xxx.xxx.xxx`), i.e. the enterprise number with BE prefix; mod-97 checksum over the 10 digits (`97 − (first 8 digits mod 97) = last 2`). **BE1-prefixed numbers also valid** since ~2024 — do not hard-code BE0 | high |
| Peppol participant scheme ID | **0208 = KBO/BCE** for the enterprise number; **9925 = BE:VAT** for the VAT number. **CORRECTION to the task assumption: 0106 is the DUTCH KvK, not Belgium.** | high |
| accountNumber kind | **iban** (BE IBAN: `BE` + 2 check + 12 digits, 16 chars) | high |

Sources:
- https://lookuptax.com/docs/tax-identification-number/belgium-tax-id-guide (formats, checksum, BE1, e-invoicing link)
- https://economie.fgov.be/en/themes/enterprises/crossroads-bank-enterprises (CBE/KBO register; public search https://kbopub.economie.fgov.be/kbopub/zoeknummerform.html)
- https://docs.peppol.eu/edelivery/codelists/old/v8.5/Peppol%20Code%20Lists%20-%20Participant%20identifier%20schemes%20v8.5.html (0106 = NL KvK)
- https://wiki.dolibarr.org/index.php/Module_Peppibar (BE: 9925 VAT, 0208 BCE/KBO)
- https://www.invoicenavigator.eu/glossary/scheme-id (0208 = Belgian KBO)

## 4. Legal forms (post-2019 WVV/CSA)

| Slug (NL convention) | Dutch | French | Notes |
|---|---|---|---|
| `bv` | Besloten vennootschap (BV) | Société à responsabilité limitée (SRL) | **SME standard**; no minimum capital (since 2019) |
| `nv` | Naamloze vennootschap (NV) | Société anonyme (SA) | public limited, listed large caps |
| `vzw` | Vereniging zonder winstoogmerk (VZW) | Association sans but lucratif (ASBL) | non-profit |
| `cv` | Coöperatieve vennootschap (CV) | Société coopérative (SC) | co-operative (CVBA/SCRL) |
| `eenmanszaak` | Eenmanszaak | Entreprise en nom propre / personne physique | sole proprietorship, natural person |

Confidence: high. Sources: https://www.expanship.com/be/blog/types-of-companies-in-belgium ; https://en.wikipedia.org/wiki/List_of_legal_entity_types_by_country ; https://www.hangark.be/blog/hoe-kies-je-de-juiste-rechtsvorm-voor-je-bedrijf-in-belgie

## 5. Chart of accounts — PCN-BE minimum plan (AR 12-09-1983)

Source (full official list, FR): https://plancomptablebelge.be/ (plan "tiré intégralement de l'arrêté royal du 12/09/1983, adapté au 01/01/2016 par AR 18/12/2015"); official CNC page: https://www.cnc-cbn.be/fr/node/2250.

**Structure:** 9 classes (1–9). Codes are **3-digit primary accounts** (e.g. 100, 411, 451), with **4-digit subdivisions** where the plan defines them (e.g. 1100, 1730, 2800); deeper extension (5–6 digits, e.g. 66200) is permitted for sub-accounts. The "6-digit" reading in the task is a simplification — the minimum plan itself is 3/4-digit; bukio may left-pad (e.g. `451000`) for consistency with other profiles, but the official codes are 3–4 digits. Classes 8 (internal) and 9 (off-balance/analytical) are optional.

Mandatory minimum accounts (all companies):

- **Classe 1 — Patrimoine propre / Eigen vermogen** (equity, credit unless noted): 10 Capital (100 Capital souscrit / Geplaatst kapitaal; 101 Capital non appelé (−) / Niet-opgevraagd kapitaal), 11 Apport hors capital (1100 Primes d'émission / Uitgiftepremies), 12 Plus-values de réévaluation / Herwaarderingsmeerwaarden, 13 Réserves / Reserves (130 Réserve légale / Wettelijke reserve, 131 Réserves indisponibles, 132 Réserves immunisées / Belastingvrije reserves, 133 Réserves disponibles / Beschikbare reserves), 14 Bénéfice/perte reporté(e) / Overgedragen winst/verlies (140 Bénéfice reporté, 141 Perte reportée — debit), 15 Subsides en capital / Kapitaalsubsidies
- **16 Provisions et impôts différés / Voorzieningen en uitgestelde belastingen** (liability, credit): 160 pensions, 161 charges fiscales, 162 grosses réparations, 163 environnement, 164–165 autres, 168 Impôts différés
- **17 Dettes à plus d'un an / Schulden op meer dan één jaar** (liability, credit): 170 Emprunts subordonnés / Achtergestelde leningen, 171 Emprunts obligataires, 172 Dettes de location-financement / Leasingschulden en soortgelijke, 173 Etablissements de crédit / Kredietinstellingen, 174 Autres emprunts, 175 Dettes commerciales / Handelsschulden (1750 Fournisseurs), 176 Acomptes reçus, 178 Cautionnements reçus, 179 Dettes diverses
- **Classe 2 — Actifs immobilisés / Vaste activa** (asset, debit): 20 Frais d'établissement / Oprichtingskosten, 21 Immobilisations incorporelles (210 Frais de développement / Ontwikkelingskosten, 211 Concessions, brevets, licences / Concessies, octrooien, licenties, 212 Goodwill), 22 Terrains et constructions / Terreinen en gebouwen (220 Terreins, 221 Constructions / Gebouwen), 23 Installations, machines et outillage / Installaties, machines en uitrusting (230 Installations, 231 Machines, 232 Outillage / Uitrusting), 24 Mobilier et matériel roulant / Meubilair en rollend materieel (240 Mobilier, matériel de bureau et informatique / Meubilair, kantooruitrusting en informatica, 241 Matériel roulant / Rollend materieel), 25 En location-financement, 26 Autres immobilisations corporelles, 27 En cours et acomptes versés, 28 Immobilisations financières / Financiële vaste activa (280 Participations / Deelnemingen, 281/283 Créances, 284 Autres actions, 285 Autres créances, 288 Cautionnements versés), 29 Créances à plus d'un an / Vorderingen op meer dan één jaar (290 Créances commerciales / Handelsvorderingen)
- **Classe 3 — Stocks / Voorraden** (asset, debit): 30 Matières premières / Grond- en hulpstoffen, 31 Fournitures / Leveringen, 32 En-cours de fabrication / Goederen in bewerking, 33 Produits finis / Gereed product, 34 Marchandises / Handelsgoederen, 35 Immeubles destinés à la vente, 36 Acomptes versés sur achats, 37 Commandes en cours d'exécution / Bestellingen in uitvoering. (Each with xx0 Valeur d'acquisition / Aanschaffingswaarde and xx9 Réductions de valeur (−) / Geboekte waardeverminderingen.)
- **Classe 4 — Créances et dettes à un an au plus / Vorderingen en schulden op ten hoogste één jaar**:
  - 40 Créances commerciales / Handelsvorderingen (asset): 400 Clients / Handelsdebiteuren, 401 Effets à recevoir, 404 Produits à recevoir, 406 Acomptes versés, 407 Créances douteuses / Dubieuze debiteuren, 409 Réductions de valeur (−)
  - 41 Autres créances (asset): 410 Capital appelé non versé, **411 TVA à récupérer / Te recupereren BTW**, 412 Impôts et précomptes à récupérer, 414 Produits à recevoir, 416 Créances diverses / Diverse vorderingen, 417 Créances douteuses, 418 Cautionnements versés, 419 Réductions de valeur (−)
  - 42 Dettes à plus d'un an échéant dans l'année / Schulden op meer dan één jaar die binnen het jaar vervallen (liability, credit)
  - 43 Dettes financières / Financiële schulden (credit): 430–432, 433 Etablissements de crédit – dettes en compte courant / Kredietinstellingen – schulden in rekening-courant, 439 Autres emprunts
  - 44 Dettes commerciales / Handelsschulden (credit): 440 Fournisseurs / Leveranciers, 441 Effets à payer, 444 Factures à recevoir / Te ontvangen facturen
  - 45 Dettes fiscales, salariales et sociales (credit): 450 Dettes fiscales estimées, **451 TVA à payer / Te betalen BTW**, 452 Impôts et taxes à payer, 453 Précomptes retenus / Ingehouden voorheffingen, 454 ONSS / RSZ, 455 Rémunérations / Bezoldigingen, 456 Pécules de vacances / Vakantiegeld, 459 Autres dettes sociales
  - 46 Acomptes reçus sur commandes / Ontvangen vooruitbetalingen op bestellingen (credit)
  - 47 Dettes découlant de l'affectation du résultat / Schulden voortvloeiend uit de resultaatverdeling (470–473, credit)
  - 48 Dettes diverses / Diverse schulden (480, 488, 489, credit)
  - 49 Comptes de régularisation / Overlopende rekeningen: 490 Charges à reporter / Over te dragen kosten (asset), 491 Produits acquis / Verkregen opbrengsten (asset), 492 Charges à imputer / Toe te rekenen kosten (liability), 493 Produits à reporter / Over te dragen opbrengsten (liability), 499 Comptes d'attente / Wachtrekeningen
- **Classe 5 — Valeurs disponibles / Beschikbare waarden** (asset, debit): 50–53 Placements de trésorerie / Geldbeleggingen (530–532 Dépôts à terme / Termijndeposito's), 54 Valeurs échues à l'encaissement, **55 Etablissements de crédit / Kredietinstellingen** (550–559 comptes courants, …0 zichtrekeningen, …1 chèques émis (−)), 56 Office des chèques postaux (legacy), **57 Caisses / Kas** (570–577 Caisses-espèces, 578 Caisses-timbres / Kaszegels), **58 Virements internes / Interne overboekingen**
- **Classe 6 — Charges / Kosten** (expense, debit): 60 Approvisionnements et marchandises (600 Achats de matières premières, 601 Fournitures, 602 Services, travaux et études, 604 Achats de marchandises / Aankopen van handelsgoederen, 608 Remises obtenues (−), 609 Variations des stocks / Wijzigingen in de voorraden), 61 Services et biens divers / Diensten en diverse goederen (617 intérim, 618 rémunérations administrateurs hors contrat), 62 Rémunérations, charges sociales et pensions / Bezoldigingen, sociale lasten en pensioenen (620 Rémunérations / Bezoldigingen, 621 Cotisations patronales / Werkgeversbijdragen, 622 Primes patronales extra-légales, 623 Autres frais de personnel / Overige personeelskosten, 624 Pensions), 63 Amortissements, réductions de valeur et provisions (630 Dotations aux amortissements / Afschrijvingen, 631–634 Réductions de valeur / Waardeverminderingen, 635–638 Provisions / Voorzieningen), 64 Autres charges d'exploitation / Andere bedrijfskosten (640 Charges fiscales d'exploitation / Bedrijfsbelastingen, 641–642 Moins-values, 643–648 Diverses), 65 Charges financières / Financiële kosten (650 Charges des dettes / Kosten van schulden, 651–655, 654 Différences de change / Wisselkoersverschillen, 657–658 Diverses), 66 Charges non récurrentes / Niet-recurrente kosten, 67 Impôts sur le résultat / Belastingen op het resultaat (670–673), 68 Transferts / Overboekingen, 69 Affectations et prélèvements / Resultaatverwerking (690 Perte reportée de l'exercice précédent, 691 Affectation au capital, 692 Dotation aux réserves / Toevoeging aan de reserves, 693 Bénéfice à reporter / Over te dragen winst, 694 Rémunération du capital, 695 Administrateurs/gérants)
- **Classe 7 — Produits / Opbrengsten** (income, credit): 70 Chiffre d'affaires / Omzet (700–707 Ventes et prestations / Verkopen en dienstverleningen, 708 Remises accordées (−)), 71 Variation des stocks / Wijzigingen in de voorraden, 72 Production immobilisée / Geactiveerde productie, 74 Autres produits d'exploitation / Andere bedrijfsopbrengsten (740 Subsides d'exploitation / Exploitatiesubsidies, 741–742 Plus-values, 743–749 Divers), 75 Produits financiers / Financiële opbrengsten (750 Produits des immobilisations financières, 751 Produits des actifs circulants, 752–755, 754 Différences de change, 756–759 Divers), 76 Produits non récurrents / Niet-recurrente opbrengsten, 77 Régularisations d'impôts / Regularisatie van belastingen, 78 Prélèvements / Onttrekkingen

### Proposed default chart (43 accounts) for `be.js` `reporting.defaultChart`

Codes verbatim from the AR 12-09-1983 minimum list. `name` = French label (plan language), NL label as second label. `taxonomyCode` null until the PCN taxonomy engine (B-milestone, same as LU).

| Code | FR name | NL name | Type | Normal balance |
|---|---|---|---|---|
| 100 | Capital souscrit | Geplaatst kapitaal | equity | credit |
| 130 | Réserve légale | Wettelijke reserve | equity | credit |
| 133 | Réserves disponibles | Beschikbare reserves | equity | credit |
| 140 | Bénéfice reporté | Overgedragen winst | equity | credit |
| 141 | Perte reportée | Overgedragen verlies | equity | debit |
| 160 | Provisions pour risques et charges | Voorzieningen voor risico's en kosten | liability | credit |
| 172 | Dettes de location-financement | Leasingschulden en soortgelijke | liability | credit |
| 173 | Etablissements de crédit (>1 an) | Kredietinstellingen (>1 jaar) | liability | credit |
| 200 | Frais d'établissement | Oprichtingskosten | asset | debit |
| 211 | Concessions, brevets, licences | Concessies, octrooien, licenties | asset | debit |
| 221 | Constructions | Gebouwen | asset | debit |
| 230 | Installations | Installaties | asset | debit |
| 231 | Machines | Machines | asset | debit |
| 240 | Mobilier, matériel de bureau et informatique | Meubilair, kantooruitrusting en informatica | asset | debit |
| 241 | Matériel roulant | Rollend materieel | asset | debit |
| 280 | Immobilisations financières | Financiële vaste activa | asset | debit |
| 300 | Matières premières | Grond- en hulpstoffen | asset | debit |
| 340 | Marchandises | Handelsgoederen | asset | debit |
| 400 | Clients | Handelsdebiteuren | asset | debit |
| 407 | Créances douteuses | Dubieuze debiteuren | asset | debit |
| 411 | TVA à récupérer | Te recupereren BTW | asset | debit |
| 412 | Impôts et précomptes à récupérer | Te recupereren belastingen en voorheffingen | asset | debit |
| 416 | Créances diverses | Diverse vorderingen | asset | debit |
| 490 | Charges à reporter | Over te dragen kosten | asset | debit |
| 422 | Dettes à plus d'un an échéant dans l'année | Schulden >1 jaar vervallend binnen het jaar | liability | credit |
| 433 | Etablissements de crédit – dettes en compte courant | Kredietinstellingen – schulden in rekening-courant | liability | credit |
| 440 | Fournisseurs | Leveranciers | liability | credit |
| 444 | Factures à recevoir | Te ontvangen facturen | liability | credit |
| 451 | TVA à payer | Te betalen BTW | liability | credit |
| 452 | Impôts et taxes à payer | Te betalen belastingen en taksen | liability | credit |
| 454 | ONSS – Office national de sécurité sociale | RSZ – Rijksdienst voor sociale zekerheid | liability | credit |
| 455 | Rémunérations | Bezoldigingen | liability | credit |
| 492 | Charges à imputer | Toe te rekenen kosten | liability | credit |
| 493 | Produits à reporter | Over te dragen opbrengsten | liability | credit |
| 550 | Banques – comptes courants | Kredietinstellingen – zichtrekeningen | asset | debit |
| 570 | Caisses-espèces | Kas | asset | debit |
| 600 | Achats de matières premières | Aankopen van grondstoffen | expense | debit |
| 604 | Achats de marchandises | Aankopen van handelsgoederen | expense | debit |
| 61 | Services et biens divers | Diensten en diverse goederen | expense | debit |
| 620 | Rémunérations | Bezoldigingen | expense | debit |
| 621 | Cotisations patronales d'assurances sociales | Werkgeversbijdragen sociale verzekeringen | expense | debit |
| 630 | Dotations aux amortissements | Afschrijvingen | expense | debit |
| 64 | Autres charges d'exploitation | Andere bedrijfskosten | expense | debit |
| 650 | Charges des dettes | Kosten van schulden | expense | debit |
| 67 | Impôts sur le résultat | Belastingen op het resultaat | expense | debit |
| 700 | Ventes et prestations de services | Verkopen en dienstverleningen | income | credit |
| 74 | Autres produits d'exploitation | Andere bedrijfsopbrengsten | income | credit |
| 75 | Produits financiers | Financiële opbrengsten | income | credit |

(46 rows incl. header block; drop 200 + 412 if a tighter 41-account chart is preferred. Optional additions per firm: 1100 Primes d'émission, 131/132 reserves, 1750 Fournisseurs >1 an, 460 Acomptes reçus, 530–532 Term deposits.)

## 6. VAT control accounts (BE convention)

| Role | Account | FR label | NL label | Type / balance | Confidence |
|---|---|---|---|---|---|
| Input VAT (récupérable) | **411** | TVA à récupérer | Te recupereren BTW | asset / debit | high (official PCMN) |
| Output VAT (due) | **451** | TVA à payer | Te betalen BTW | liability / credit | high (official PCMN) |
| Settlement | none separate — net of 451/411 is settled to/from bank (55) on payment; the tax authority's Intervat periodical return nets input vs output | | | | medium (convention; the minimum plan has no dedicated settlement account) |

Sources: https://plancomptablebelge.be/ (411 under 41, 451 under 45); https://expert.taxwin.be/fr/tw_src_off_fisc/document/art.cnc.132-007-fr (CNC Avis 132-7 uses 411 TVA à récupérer / 440 Fournisseurs).

Profile mapping suggestion (per LU pattern):
- `tax.accounts.ledger` = `[{411, asset, debit}, {451, liability, credit}]`
- `fileDefault: '451'` (net VAT payable posted to 451, paid from 550)
- `differenceDefault`: no rounding-difference account designated by the plan → default to 64/648 (Charges d'exploitation diverses) per convention; **unverified** — flag as per-firm convention
- `afTeDragenName: 'TVA à payer'`

## 7. Statutory accounts (jaarrekening)

- Filed with the **NBB Central Balance Sheet Office** (Balanscentrale/Centrale des bilans) within **30 days of AGM approval**, at the latest **7 months after the end of the financial year** (e.g. 12-31 FYE → 31 July). Late filing penalties kick in from the 9th month.
- Layout: full vs abbreviated ("schema verkort" / "schéma abrégé") per size criteria — **B-milestone** in bukio (reporting.format, e.g. `be-nbb-verkort`); just note the regime here.
- Sources: https://www.nbb.be/en/central-balance-sheet-office/preparation-and-filing/when-and-how-file/filing-deadline (official, high); https://news.pwc.be/2026-belgian-statutory-and-tax-compliance-deadlines-key-filing-reminders/ (high)

## 8. Fiscal year end

- **Default 12-31**; any date allowed (accounts must be closed every 12 months). Confidence: high (universal practice; PwC calendar uses 12-31 examples; AGM/approval cycle implies any FYE).

## 9. Banking

- **SEPA** zone member; IBAN format `BE` + 2 check digits + 12 digits (16 chars). Confidence: high (standard, https://www.europeanpaymentscouncil.eu/ context).
- **CAMT.053**: offered by Belgian banks as standard ISO 20022 statement format (bukio already imports CAMT.053); **availability is bank-dependent** — not verified per-bank this run. Confidence: medium.
- **CODA** (Coded Statement, the legacy Belgian-specific bank statement format, widely used by BE accounting software): **low confidence / unverified this run** — worth a B-milestone check if CODA import is ever needed.

## 10. Compliance calendar (SME, 12-31 FYE)

| When | What | Confidence |
|---|---|---|
| Monthly, by the **20th** | VAT return + payment (Intervat) for monthly filers | high |
| Quarterly, by the **25th** | VAT return + payment for quarterly filers (≤ €2.5M turnover) | high |
| **31 March** | Annual client listing (jaarlijkse listing) for prior year, incl. franchise businesses (nihil if none) | high |
| ≤ **30 days after AGM**, max **7 months after FYE** | Annual accounts filed with NBB (12-31 FYE → 31 July) | high |
| ~**30 September** (9 months after FYE) | Corporate income tax return (vennootschapsbelasting) for 12-31 FYE (AY 2026: 30 Sep 2026) | high |
| ~**29/30 June** | Fee forms 281.50 (commissions/fees paid) via Belcotax-on-web | high |
| Not researched (out of scope) | Payroll/social security (RSZ/ONSS monthly declarations), prepayments (acomptes) — flag unverified | — |

Source: https://news.pwc.be/2026-belgian-statutory-and-tax-compliance-deadlines-key-filing-reminders/ (high)

## 11. e-Invoicing — **CONFIRMED**

- **Mandatory B2B e-invoicing for all domestic transactions between Belgian VAT-registered businesses since 1 January 2026**, exchanged via the **Peppol network** (no real-time clearance/tax-authority clearance — decentralized).
- Format: **Peppol BIS Billing 3.0**, i.e. **EN 16931**-compliant **UBL 2.1**. Supplier VAT number (BE + 10 digits) is mandatory; structurally invalid VAT numbers are rejected by access points.
- Enforcement: tolerance/triplement period Jan–Mar 2026; sanctions enforcement resumed **1 April 2026**.
- Sources:
  - https://ec.europa.eu/digital-building-blocks/sites/spaces/DIGITAL/pages/467108877/eInvoicing+in+Belgium (European Commission, high)
  - https://peppolvalidator.com/peppol-belgium (high)
  - https://lookuptax.com/docs/tax-identification-number/belgium-tax-id-guide (Peppol BIS 3.0 UBL 2.1, tolerance period, high)
  - https://www.zoneandco.com/articles/e-invoicing-regulations-in-belgium-what-finance-teams-need-to-know-before-2026 (high)
  - https://www.vatupdate.com/2026/01/10/comprehensive-vat-guide-belgium-2026/ (high)
- Profile mapping: `documents.eInvoicing: 'peppol-bis-3.0'` (same as LU).

## 12. Closing accounts (BE convention)

- **CORRECTION to the task assumption:** 12x is NOT the result account — **12 Plus-values de réévaluation / Herwaarderingsmeerwaarden** (revaluation surpluses). The annual result is closed to **14 Bénéfice/perte reporté(e)** — **140 Overgedragen winst** (credit) or **141 Overgedragen verlies** (debit) — via the class-69 result-processing accounts (**693 Bénéfice à reporter / Over te dragen winst**; **690 Perte reportée** on a loss). Dividends/allocations hit 47 (Dettes découlant de l'affectation du résultat / Schulden voortvloeiend uit de resultaatverdeling) via 691/692/694/695.
- Confidence: high (official PCMN class 1 + class 69; https://plancomptablebelge.be/).

## 13. Currency / locale / dates

| Item | Value | Confidence |
|---|---|---|
| Currency | **EUR** | high |
| Locale | **nl-BE** default (fr-BE alternative; labels in this brief are FR primary + NL secondary, matching the plan's bilingual reality) | high |
| Date format | **dd/mm/yyyy** | high |

---

## Flags / unverified items

1. **Exact franchise invoice wording** (exemption mention on invoices under art. 56bis WBTW) — not extracted this run; FPS brochure: https://www.minfin.fgov.be/myminfin-web/pages/public/fisconet/document/2b18be46-ea25-4430-93e0-3c8df0bf7fb9.
2. **Peppol scheme 0106** — confirmed WRONG for BE (0106 = NL KvK); use **0208 (KBO)** or **9925 (BE:VAT)**.
3. **Quarterly VAT payment due date** (25th per Marosa/Meridian vs historic 20th) — medium confidence; verify against Intervat guidance before coding.
4. **CAMT.053 per-bank availability** and **CODA** legacy format — unverified this run.
5. **VAT rounding/`differenceDefault` account** — no plan-mandated account; convention only.
6. **Payroll/social-security monthly declaration deadline** (RSZ/ONSS) — out of research scope, not verified.
7. Franchise scheme effective date is **2025**, not 2024 (task hypothesis corrected).
8. All PCMN labels verified against one comprehensive secondary source (plancomptablebelge.be, itself "tiré intégralement" of the AR); the official CNC text (https://www.cnc-cbn.be/fr/node/2250) was not extracted — flag for a final spot-check before shipping be.js.

## Research notes

- Runs: 14 web_search/web_extract calls, 0 browser calls (per budget). No timeouts.
- Previous runs timed out on browser fetches — avoided entirely this time.
