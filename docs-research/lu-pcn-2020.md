# Luxembourg PCN 2020 (Plan Comptable Normalisé) — Research Brief for bukio-cli `lu.js` jurisdiction profile

**Research date:** 2026-08-15 · **Prepared for:** bukio-cli jurisdiction-profile layer (reference: `src/jurisdictions/nl.js`)
**Scope:** PCN 2020 account codes (with French labels) for an SME default chart, VAT accounts, result/equity conventions, and statutory (LSC) mapping. Every code below was verified against the official annex or a reputable secondary source; confidence is marked per item.

## 0. Legal basis & sources

| # | Source | URL | Confidence |
|---|--------|-----|-----------|
| S1 | **Official** — Règlement grand-ducal du 12 septembre 2019 déterminant le contenu du plan comptable normalisé visé à l'art. 12 du Code de commerce (Mémorial A n° 631, 23.09.2019) — includes the full PCN annex + tableau de passage | `https://legilux.public.lu/eli/etat/leg/rgd/2019/09/12/a631/jo` (PDF: `http://data.legilux.public.lu/file/eli-etat-leg-rgd-2019-09-12-a631-jo-fr-pdf.pdf`) | **High** (official) |
| S2 | KPMG Luxembourg — "Plan Comptable Normalisé Luxembourgeois" (Nov 2019, full PCN reprint) | `https://assets.kpmg.com/content/dam/kpmg/lu/pdf/luxembourg-standard-chart-accounts-FR-web.pdf` | High |
| S3 | Guichet.lu — "The chart of accounts for businesses" (scope, structure, who is subject) | `https://guichet.public.lu/en/entreprises/gestion-juridique-comptabilite/comptable/enregistrement/plan-comptable.html` | High (official portal) |
| S4 | Odoo 17 `l10n_lu` localization chart data (747 accounts, PCN 2020 codes verbatim — practical confirmation of a seeded chart) | `https://raw.githubusercontent.com/odoo/odoo/17.0/addons/l10n_lu/data/template/account.account-lu.csv` | High (reputable software implementation) |
| S5 | CSSF — publication notice of the GDR of 12 Sept 2019 (scope incl. PSF) | `https://www.cssf.lu/en/2020/01/publication-of-gdr-of-12-september-2019-determining-the-content-of-the-standard-chart-of-accounts-and-of-qa-cnc-19-019-categorisation-of-firms-art-36-lrcs/` | Medium |
| S6 | Advena — "PCN 2020 le guide" (secondary; **claims a class 8 — contradicted by S1**, see §7) | `https://advena.lu/journal/plan-comptable-normalise-au-luxembourg-pcn-2020-le-guide` | Low/Medium |

### Key legal facts (verified, S1)
- **RGD 12 Sept 2019** replaced the RGD 10 June 2009. Applies to financial years starting **on/after 1 Jan 2020** (Art. 13). Mandatory for SA, Sàrl, Sàrl-S, SE, SC, SCA, SCS, SNC, GEIE/GIE, branches of foreign traders, soparfis (S3, S5).
- Exempt: sole traders & SNC/SCS with turnover < €100,000 excl. VAT; SCS special; credit institutions; insurers; CSSF-supervised financial sector (except support PSF); IFRS filers (S3).
- **Exactly 7 classes** (Art. 3 S1): Classe 1 capitaux propres/provisions/dettes financières · 2 frais d'établissement & actifs immobilisés · 3 stocks & en-cours · 4 comptes de tiers · 5 comptes financiers · 6 charges · 7 produits. **There is NO class 8 in the official PCN** (see §7).
- Two account kinds (Art. 7 S1): **comptes de regroupement (R)** — aggregation only; **comptes d'imputation (I)** — the only accounts businesses may post to. **bukio should only seed "I" accounts as postable.**
- ⚠️ **Codes are NOT uniformly 5 digits.** The official annex uses hierarchical codes of 2–6 digits (e.g. `101`, `516`, `601` are 3-digit *imputation* accounts; `4011` is 4; `421611` is 6). The "5-digit" claim common in marketing material is a simplification (S6). Use the PCN codes verbatim.
- Filing: companies file the balance of PCN imputation accounts + a *tableau de passage* (reconciliation table) via **eCDF** (electronic gathering of financial data) to the RCS; the FAIA audit file (LU SAF-T) also uses PCN codes (S3, S6).

## 1. Core SME chart (cash, bank, debtors, creditors, capital, result)

All codes are imputation accounts (I) verified in S1 (annex) and cross-confirmed by S4 (Odoo) unless noted.

| PCN code | French label (PCN) | Class / type | Statutory line (S1 tableau de passage) | Source |
|---|---|---|---|---|
| 101 | Capital souscrit | 1 · equity | Passif A.I. | S1, S4 |
| 104 | Capital des entreprises individuelles, des sociétés de personnes et assimilées | 1 · equity | Passif A.I. | S1, S4 |
| 142 | Résultat de l'exercice | 1 · equity | Passif A.VI. | S1, S4 |
| 1411 | Résultats reportés en instance d'affectation | 1 · equity | Passif A.V. | S1, S4 |
| 1412 | Résultats reportés (affectés) | 1 · equity | Passif A.V. | S1, S4 |
| 131 | Réserve légale | 1 · equity | Passif A.IV.1. | S1, S4 |
| 133 | Réserves statutaires | 1 · equity | Passif A.IV.3. | S1, S4 |
| 1381 | Autres réserves disponibles | 1 · equity | Passif A.IV.4.a) | S1, S4 |
| 1941 | Dettes envers des établissements de crédit — durée résiduelle ≤ 1 an | 1 · liability | Passif C.2.a) | S1 |
| 1942 | Dettes envers des établissements de crédit — durée résiduelle > 1 an | 1 · liability | Passif C.2.b) | S1 |
| 4011 | Clients (créances résultant de ventes et prestations, ≤ 1 an) | 4 · asset | Actif D.II.1.a) | S1, S4 |
| 4014 | Clients – Factures à établir | 4 · asset | Actif D.II.1.a) | S1 |
| 4019 | Corrections de valeur (clients) | 4 · contra-asset | Actif D.II.1.a) | S1 |
| 44111 | Fournisseurs (dettes sur achats et prestations, ≤ 1 an) | 4 · liability | Passif C.4.a) | S1, S4 |
| 44112 | Fournisseurs – Factures non parvenues | 4 · liability | Passif C.4.a) | S1, S4 |
| 44113 | Fournisseurs débiteurs | 4 · asset | Actif D.II.1.a) | S1 |
| 5131 | Banques et comptes chèques postaux (CCP) : avoirs | 5 · asset | Actif D.IV. | S1, S4 |
| 5132 | Banques et CCP : découverts | 5 · liability | Passif C.2.a) | S1, S4 |
| 516 | Caisse | 5 · asset | Actif D.IV. | S1, S4 |
| 4714 | Dettes envers le personnel (≤ 1 an) | 4 · liability | Passif C.8.c)i) | S1 |
| 4621 | Centre Commun de Sécurité Sociale (CCSS) — dettes sécurité sociale | 4 · liability | Passif C.8.b) | S1 |
| 4712 | Dettes envers associés et actionnaires (autres qu'entreprises liées) | 4 · liability | Passif C.8.c)i) | S1 |
| 481 | Charges à reporter | 4 · asset (prepayment) | Actif E. | S1 |
| 482 | Produits à reporter | 4 · liability (deferral) | Passif D. | S1 |

Useful fixed-asset (classe 2) and stock (classe 3) codes for a small chart (S1):

| PCN code | French label | Statutory line |
|---|---|---|
| 201 | Frais de constitution et de premier établissement | Actif B. |
| 213 | Fonds de commerce (acquis à titre onéreux) | Actif C.I.3. |
| 21213 | Licences informatiques (acquis à titre onéreux) | Actif C.I.2.a) |
| 22111 | Terrains au Luxembourg | Actif C.II.1. |
| 22131 | Constructions / Bâtiments au Luxembourg | Actif C.II.1. |
| 2221 | Installations techniques | Actif C.II.2. |
| 2231–2238 | Autres installations, outillage et mobilier (y compris matériel roulant) | Actif C.II.3. |
| 301 | Stocks de matières premières | Actif D.I.1. |
| 361 | Stocks de marchandises | Actif D.I.3. |

## 2. VAT accounts (exact PCN codes)

VAT lives in **classe 4, under the AED** (Administration de l'Enregistrement, des Domaines et de la TVA) — the Luxembourg VAT authority. Verified S1 (annex) + S4.

| PCN code | French label | Nature | Statutory line (S1) | Source |
|---|---|---|---|---|
| **421611** | **TVA en amont** (input VAT, deductible) | asset (current) | Actif D.II.4.a) | S1, S4 |
| 421612 | TVA à recevoir (credit balance of the return) | asset (current) | Actif D.II.4.a) | S1, S4 |
| 421613 | TVA acomptes versés | asset (current) | Actif D.II.4.a) | S1 |
| 421618 | TVA – Autres créances | asset (current) | Actif D.II.4.a) | S1 |
| **461411** | **TVA en aval** (output VAT collected) | liability (current) | Passif C.8.a) | S1, S4 |
| **461412** | **TVA à payer** (debit balance of the return) | liability (current) | Passif C.8.a) | S1, S4 |
| 461413 | TVA acomptes reçus | liability (current) | Passif C.8.a) | S1 |
| 461418 | TVA – Autres dettes | liability (current) | Passif C.8.a) | S1 |
| 6462 | TVA non récupérable (non-deductible VAT as an expense) | classe 6 · expense | P&L 8. | S1, S4 |
| 421811 / 46151 | TVA étrangères (receivable / payable) | 4 | D.II.4.a) / C.8.a) | S1 |

- **Luxembourg VAT rates** (S4-based Odoo 19 docs, medium-high confidence): standard **17 %**, reduced **14 %** (catering/hospitality) and **8 %**, super-reduced **3 %**, plus 0 %/exempt/reverse-charge intra-community. Tax returns are filed with the AED on **eCDF** (XML).
- Practical confirmation that 421611 = input VAT and 461411 = output VAT is also given by S4 (`"TVA en amont"` = "VAT paid and recoverable", `"TVA en aval"` = "VAT received").

## 3. Revenue accounts (classe 7) — main sales accounts

All verified S1 + S4. The whole class 7 maps to P&L lines via the tableau de passage.

| PCN code | French label | P&L line (S1) | Source |
|---|---|---|---|
| 70 | Montant net du chiffre d'affaires (regroupement R — not postable) | 1. | S1 |
| 7021 | Ventes de produits finis | 1. | S1, S4 |
| 7033 | Prestations de services non visées ci-dessus | 1. | S1, S4 |
| 70321 | Revenus de location immobilière | 1. | S1 |
| 7061 | Ventes de marchandises | 1. | S1, S4 |
| 708 | Autres éléments du chiffre d'affaires | 1. | S1, S4 |
| 7092–7099 | Rabais, remises et ristournes (RRR) accordés (contra-revenue) | 1. | S1 |
| 7488 | Produits d'exploitation divers | 4. | S1 |
| 7421 | Revenus de location à titre accessoire | 4. | S1 |
| 7451–7458 | Subventions d'exploitation | 4. | S1 |
| 7481 | Indemnités d'assurance | 4. | S1 |
| 7582 | Autres produits financiers – autres | 11.b) | S1 |
| 771/772 | Régularisations IRC / ICC | 15. | S1 |

## 4. Expense accounts (classe 6) — codes a small chart needs

All verified S1 + S4.

| PCN code | French label | Sub-class | P&L line | Source |
|---|---|---|---|---|
| 601 | Achats de matières premières | 60 | 5.a) | S1, S4 |
| 6061 | Achats de marchandises | 60 | 5.a) | S1, S4 |
| 6071 / 60761 | Variation des stocks (matières premières / marchandises) | 60 | 5.a) | S1 |
| 6091–6099 | RRR obtenus (contra-purchases) | 60 | 5.a) | S1 |
| 61112 | Loyers et charges locatives — Constructions / Bâtiments | 61 · rent | 5.b) | S1, S4 |
| 61123 | Locations mobilières — Matériel roulant | 61 | 5.b) | S1 |
| 6113 | Charges locatives et de copropriété | 61 | 5.b) | S1 |
| 6114 | Leasing financier immobilier | 61 | 5.b) | S1 |
| 61333 | Frais de comptes et commissions bancaires | 61 | 5.b) | S1 |
| 61342 | Honoraires comptables, fiscaux, d'audit et assimilés | 61 | 5.b) | S1, S4 |
| 61341 | Honoraires juridiques, de contentieux et assimilés | 61 | 5.b) | S1 |
| 6141–6148 | Primes d'assurance | 61 | 5.b) | S1 |
| 61532 | Frais de télécommunication | 61 | 5.b) | S1 |
| 61845 | Électricité (non incorporée) | 61 | 5.b) | S1 |
| 6188 | Autres charges externes diverses | 61 | 5.b) | S1 |
| 62111 | Salaires de base (Frais de personnel) | 62 | 6.a) | S1, S4 |
| 62114 | Gratifications, primes et commissions | 62 | 6.a) | S1 |
| 62116 | Indemnités de licenciement | 62 | 6.a) | S1 |
| 6231 | Charges sociales couvrant les pensions (part patronale) | 62 | 6.b)i) | S1, S4 |
| 6232 | Autres charges sociales (y inclus maladie, accident) | 62 | 6.b)ii) | S1, S4 |
| 6248 | Autres frais de personnel non visés ci-dessus | 62 | 6.c) | S1 |
| **6333** | **DCV sur autres installations, outillage et mobilier (y compris matériel roulant)** ← main depreciation account | 63 | 7.a) | S1, S4 |
| 63313 | DCV sur constructions / bâtiments | 63 | 7.a) | S1 |
| 6332 | DCV sur installations techniques et machines | 63 | 7.a) | S1 |
| 6322 | DCV sur concessions, brevets, licences, marques | 63 | 7.a) | S1 |
| 6351 | DCV sur créances résultant de ventes et prestations de services | 63 | 7.b) | S1 |
| 6461 | Impôt foncier | 64 | 8. | S1 |
| 6466 | Taxes sur les véhicules | 64 | 8. | S1 |
| 6488 | Charges d'exploitation diverses | 64 | 8. | S1, S4 |
| 6492 | Dotations aux provisions d'exploitation | 64 | 8. | S1 |
| 65521 | Intérêts sur comptes bancaires | 65 · financial | 14.b) | S1, S4 |
| 65522 | Intérêts bancaires sur opérations de financement | 65 | 14.b) | S1, S4 |
| 6553 | Intérêts sur dettes commerciales | 65 | 14.b) | S1 |
| 65582 | Intérêts sur autres emprunts et dettes – autres | 65 | 14.b) | S1 |
| 6561/6562 | Pertes de change | 65 | 14. | S1 |
| 6582 | Autres charges financières – autres | 65 | 14.b) | S1 |
| 6711 | Impôt sur le revenu des collectivités (IRC) – exercice courant | 67 | 15. | S1, S4 |
| 6721 | Impôt commercial communal (ICC) – exercice courant | 67 | 15. | S1, S4 |
| 6811 | Impôt sur la fortune (IF) – exercice courant | 68 | 17. | S1 |
| 682 | Taxe d'abonnement | 68 | 17. | S1 |

> Note: the PCN uses **"DCV" (dotations aux corrections de valeur)** for depreciation/impairment — there is no "amortissements" wording. LU fiscal depreciation is usually booked via 6333/63313/6332.

## 5. Equity / reserves (classe 1)

| PCN code | French label | Statutory line | Source |
|---|---|---|---|
| 101 | Capital souscrit | Passif A.I. | S1, S4 |
| 111 | Primes d'émission | Passif A.II. | S1, S4 |
| 131 | Réserve légale (5 % of annual profit until 10 % of capital — LSC 1915 art. 127-1, not in PCN itself) | Passif A.IV.1. | S1, S4 |
| 133 | Réserves statutaires | Passif A.IV.3. | S1, S4 |
| 1381 | Autres réserves disponibles | Passif A.IV.4.a) | S1, S4 |
| 13821 | Réserve pour l'impôt sur la fortune (IF) | Passif A.IV.4.b) | S1, S4 |
| 1411 | Résultats reportés en instance d'affectation | Passif A.V. | S1, S4 |
| 1412 | Résultats reportés (affectés) | Passif A.V. | S1, S4 |
| 142 | Résultat de l'exercice | Passif A.VI. | S1, S4 |

## 6. VAT settlement / rounding-difference convention

- **Settlement flow (practice, consistent with S1/S4 structure):** at each VAT return, 421611 (TVA en amont, debit) and 461411 (TVA en aval, credit) are netted. A credit balance (TVA due) is booked to **461412 TVA à payer** and paid to the AED; a debit balance (TVA à récupérer) to **421612 TVA à recevoir**.
- **Rounding differences:** the PCN **does not designate any specific account** for the cent/rounding difference of the VAT return (no "différence d'arrondi" account exists in the official annex — verified by full read of S1). No official source found.
  - **Confidence: LOW** — not verifiable from official sources. Practical conventions (medium confidence, from LU accounting practice as reflected in S4-era software): absorb the difference into the settlement accounts (461412/421612) or the umbrella "TVA – Autres dettes/créances" (461418/421618), or book to a small expense (e.g. 6488 / 6462-style). **Recommendation:** make the difference account configurable in `lu.js` (analogous to NL's `differenceDefault: '4700'`), defaulting to `461418` (or a dedicated sub-account), and document that the choice is a per-firm convention, not a PCN rule.
  - The PCN's only VAT-specific charge account is **6462 TVA non récupérable** (S1) — the natural home for non-deductible VAT.

## 7. Year-end / result account convention

- **The official PCN has exactly 7 classes (Art. 3, S1) — there is NO "classe 8 résultat".** The Advena guide (S6) claims "class 8 holding result accounts and special accounts"; this is **contradicted by the official RGD annex** and should not be implemented. (Confidence: official = high; the class-8 claim = low.)
- **Annual result account: `142 Résultat de l'exercice`** (classe 1, grouping `14 - Résultats`, statutory line Passif A.VI.). At year-end: close classes 6/7 to 142; after the shareholders' meeting, the appropriation is booked 142 → **1411 Résultats reportés en instance d'affectation** / **1412 Résultats reportés (affectés)** and to 131 (réserve légale) etc. (S1, S4).
- So the LU analogue of NL's 1200/1210 style is the **14x block in classe 1**, not a class 8. No separate "compte de résultat" class exists.

## 8. Statutory accounts mapping (LSC bilan & compte de profits et pertes)

The RGD annex contains the official **tableau de passage** (S1) mapping every PCN account to the LSC statutory layouts (loi modifiée du 19 décembre 2002; non-abridged and abridged columns). Class-level mapping:

### Bilan (balance sheet)
| Statutory heading (LSC) | PCN accounts (class/group) |
|---|---|
| **Actif** A. Capital souscrit non versé | 102, 103 |
| B. Frais d'établissement | 20x (201, 203, 204, 208) |
| C. Actif immobilisé — I. Incorporel | 21x (211, 2121x, 2122x, 213) |
| C. Actif immobilisé — II. Corporel | 22x (2211x, 2213x, 222x, 223x, 224x) |
| C. Actif immobilisé — III. Financier | 23x–25x (parts, créances, prêts) |
| D. Actif circulant — I. Stocks | 3x (301, 303, 311, 321, 361…) |
| D. Actif circulant — II. Créances (1. ventes & prestations · 2. entreprises liées · 3. lien de participation · 4. autres) | 40x (4011–4019, 402x), 411x, 412x, 421x, 422x |
| D. Actif circulant — III. Valeurs mobilières | 50x (501, 502, 503, 508x) |
| D. Actif circulant — IV. Avoirs en banques, CCP, chèques et encaisse | 5131, 516, 5171, 518 |
| E. Comptes de régularisation | 481, 484, 486 |
| **Passif** A.I. Capital souscrit | 101, 104, 105 |
| A.II. Primes d'émission | 11x (111–115) |
| A.III. Réserves de réévaluation | 12x (122, 123, 128) |
| A.IV. Réserves (1. légale · 2. actions propres · 3. statutaires · 4. autres) | 131, 132, 133, 1381, 1382x |
| A.V. Résultats reportés | 1411, 1412 |
| A.VI. Résultat de l'exercice | 142 |
| A.VII. Acomptes sur dividendes | 15 |
| A.VIII. Subventions d'investissement en capital | 161x, 162x, 168 |
| B. Provisions (1. pensions · 2. impôts · 3. autres) | 181, 182/183, 1881/1882 |
| C.1. Emprunts obligataires | 192x, 193x |
| C.2. Dettes envers établissements de crédit | 1941/1942 (+ 5132, 5172 for overdrafts) |
| C.3. Acomptes reçus sur commandes | 431x, 432x |
| C.4. Dettes sur achats et prestations de services | 4015/4025, 4411x, 4412x |
| C.5. Dettes représentées par effets de commerce | 442x |
| C.6. Dettes envers entreprises liées | 451x |
| C.7. Dettes envers entreprises liées par participation | 452x |
| C.8. Autres dettes (y compris dettes fiscales et sociales) | 46x (4611–4615, 462x), 471x, 472x |
| D. Comptes de régularisation | 482, 483, 485, 487 |

### Compte de profits et pertes (P&L)
| Statutory line (LSC, non-abridged) | PCN accounts |
|---|---|
| 1. Montant net du chiffre d'affaires | 70x (7021, 7033, 7061, 708, 709x) |
| 2. Variation des stocks de produits finis et d'en-cours | 71x |
| 3. Production immobilisée | 72x |
| 4. Autres produits d'exploitation | 74x |
| 5. a) Matières premières et consommables · b) Autres charges externes | 60x · 61x |
| 6. Frais de personnel | 62x |
| 7. a) DCV sur frais d'établissement, immos incorp. & corp. · b) DCV sur actifs circulants | 631x–633x · 634x–635x |
| 8. Autres charges d'exploitation | 64x |
| 9. Produits financiers — a) revenus de participations · b) revenus d'autres immos financières · c) intérêts et autres produits financiers | 75211/75213 · 75212/75214–75216 · 758x, 759x |
| 10. (a/b) revenus & plus-values de cession d'immobilisations financières | 7522x |
| 11. (a/b) autres produits financiers | 758x, 759x |
| 12. Quote-part dans le bénéfice/perte des entreprises mises en équivalence | 757, 657 |
| 13. DCV / AJV sur immobilisations financières et valeurs mobilières | 651x, 653x, 751x, 753x |
| 14. (a/b) intérêts et autres charges financières | 652x, 654x, 655x, 656x, 658x, 659x |
| 15. Impôts sur le résultat | 67x (6711, 6721, 673x, 679), 77x (771–779) |
| 16. Résultat de l'exercice | (result line — fed by 142) |
| 17. Autres impôts ne figurant pas sous les postes ci-dessus | 68x (6811, 682, 683, 688), 78x (781–788) |

> The same annex provides a second column mapping to the **abridged** (abrégé) schemes (items 1–5 aggregated; bilan letters without sub-numbers), which is what most SMEs file.

## 9. Suggested default chart for `lu.js` (mapped to NL profile shape)

Seed only **imputation (I)** accounts; fields mirror NL: `code, name, type (asset|liability|equity|income|expense), normalBalance (debit|credit), taxonomyCode` (use PCN code as taxonomy key; statutory line as extra metadata). ~30 accounts:

| code | name (FR) | type | normalBalance |
|---|---|---|---|
| 101 | Capital souscrit | equity | credit |
| 131 | Réserve légale | equity | credit |
| 133 | Réserves statutaires | equity | credit |
| 1381 | Autres réserves disponibles | equity | credit |
| 1411 | Résultats reportés en instance d'affectation | equity | credit |
| 1412 | Résultats reportés (affectés) | equity | credit |
| 142 | Résultat de l'exercice | equity | credit |
| 1941 | Dettes envers établissements de crédit (≤ 1 an) | liability | credit |
| 4011 | Clients | asset | debit |
| 421611 | TVA en amont | asset | debit |
| 421612 | TVA à recevoir | asset | debit |
| 44111 | Fournisseurs | liability | credit |
| 44112 | Fournisseurs – Factures non parvenues | liability | credit |
| 461411 | TVA en aval | liability | credit |
| 461412 | TVA à payer | liability | credit |
| 461418 | TVA – Autres dettes | liability | credit |
| 4621 | CCSS – dettes sécurité sociale | liability | credit |
| 4714 | Dettes envers le personnel | liability | credit |
| 4712 | Dettes envers associés et actionnaires | liability | credit |
| 481 | Charges à reporter | asset | debit |
| 482 | Produits à reporter | liability | credit |
| 5131 | Banques et CCP : avoirs | asset | debit |
| 5132 | Banques et CCP : découverts | liability | credit |
| 516 | Caisse | asset | debit |
| 601 | Achats de matières premières | expense | debit |
| 6061 | Achats de marchandises | expense | debit |
| 61112 | Loyers – Constructions / Bâtiments | expense | debit |
| 61342 | Honoraires comptables, fiscaux, d'audit | expense | debit |
| 62111 | Salaires de base | expense | debit |
| 6232 | Autres charges sociales | expense | debit |
| 6333 | DCV sur autres installations, outillage et mobilier | expense | debit |
| 6462 | TVA non récupérable | expense | debit |
| 6488 | Charges d'exploitation diverses | expense | debit |
| 65521 | Intérêts sur comptes bancaires | expense | debit |
| 65582 | Intérêts sur autres emprunts et dettes | expense | debit |
| 6711 | IRC – exercice courant | expense | debit |
| 6721 | ICC – exercice courant | expense | debit |
| 7021 | Ventes de produits finis | income | credit |
| 7033 | Prestations de services | income | credit |
| 7061 | Ventes de marchandises | income | credit |
| 708 | Autres éléments du chiffre d'affaires | income | credit |
| 7488 | Produits d'exploitation divers | income | credit |

## 10. Items NOT verifiable / flags

1. **VAT rounding-difference account (item 6):** no PCN account exists; official sources silent. Conventions only (461412/421612 vs 461418/421618) — **low confidence, make configurable**.
2. **"Class 8" result accounts (item 7):** claimed by one secondary source (S6) but **contradicted by the official RGD** (7 classes, Art. 3). Official answer: 142/1411/1412 in classe 1.
3. **"5-digit codes":** popular simplification; official codes are hierarchical 2–6 digits (e.g. 516, 4011, 421611).
4. Réserve légale rate (5 %/10 % of capital) comes from the **Loi modifiée du 10 août 1915 (art. 127-1)** — not from the PCN; not re-verified against the statute text here (medium confidence).
5. VAT rates 17/14/8/3 % cited from Odoo 19 localization docs (medium-high); AED confirmation not fetched.
6. The exact statutory P&L line *labels* for items 9–14 are inferred from the PCN annex numbering; the annex itself gives only the numbers in the mapping columns. For a statutory layout, pull the LSC annex (loi 19.12.2002, art. 43) before hard-coding labels.
