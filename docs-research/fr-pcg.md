# France — PCG (Plan Comptable Général) research brief for the bukio-cli FR jurisdiction profile

Status: research brief, 15 Aug 2026. Purpose: seed the France jurisdiction profile (`src/jurisdictions/fr.js`) — default SME chart, VAT accounts, statutory class mapping, VAT rates/regimes. Companion to `lu-pcn-2020.md` (same brief format).

## Regulatory context (verified)

- The French chart of accounts is the **PCG** = règlement **ANC n°2014-03** (5 June 2014), consolidated. Since 2022 it was heavily modified: **règlement ANC 2022-06** ("modernisation des états financiers", mandatory from fiscal years opening 1 Jan 2025) reduced the chart from >2,000 to ~1,600 accounts (single plan: mandatory + optional accounts, **the old "système abrégé / système développé" chart distinction disappeared**), suppressed transfer accounts **791/796/797**, restricted "résultat exceptionnel" (67/77) to major unusual events, and replaced the multiple bilan/compte de résultat models with **one bilan (tableau) + one compte de résultat (liste) model**. **Règlement ANC 2024-07** (homologated 26 Dec 2025, applicable from 1 Jan 2026) adds a mandatory **"autres fonds propres"** passif line (fonds non remboursables / avances conditionnées / droits du concédant) and details BSA/BSPCE in 104.
- Sources: https://www.compta-online.com/plan-comptable-general-pdf-ao2428 (modified 14/08/2026) — High; official consolidated chart PDF (Jan 2024): https://www.anc.gouv.fr/files/anc/files/1_Normes_fran%C3%A7aises/Reglements/Recueils/PCG_Janvier2024/PCG--1er-janvier-2024.pdf — High.
- Complete account lists (labels below) verified against https://www.dougs.fr/ressources/pcg/ per-class pages (classe-1/2/4/5/6/7 extracted 15 Aug 2026) and https://www.l-expert-comptable.com/plan-comptable (class index) — Medium-High (professional sources reproducing the official list; 205 label nuance flagged in §Flags).
- Class structure: classes 1–5 = bilan (patrimoine), classes 6–7 = compte de résultat (charges/produits), classe 8 = special accounts. Sources: https://www.l-expert-comptable.com/plan-comptable , https://www.compta-facile.com/plan-des-comptes-en-comptabilite/ — High (both consistent with PCG).

## Confidence legend

- **High** — verified on a government source (service-public.gouv.fr / impots.gouv.fr / anc.gouv.fr) or on ≥2 consistent professional PCG lists.
- **Med-High** — single professional PCG list (dougs.fr / l-expert-comptable.com / compta-facile.com) or gov snippet.
- **Med** — reputable commercial source, single occurrence.
- **Unverified** — stated from general knowledge, NOT confirmed in this research pass.

---

## 1. Core SME chart (comptes de bilan)

| Code | Label FR (official PCG) | Type | Normal balance | Source | Confidence |
|---|---|---|---|---|---|
| 101 | Capital social (sous-comptes 1011 non appelé / 1012 appelé non versé / 1013 appelé versé) | equity | credit | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 1061 | Réserve légale | equity | credit | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 1068 | Autres réserves | equity | credit | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 110 | Report à nouveau – solde créditeur | equity | credit | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 119 | Report à nouveau – solde débiteur | equity | debit | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 120 | Résultat de l'exercice – bénéfice | equity | credit | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 129 | Résultat de l'exercice – perte | equity | debit | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 401 | Fournisseurs (4011 Achats de biens et prestations de services) | liability | credit | https://www.dougs.fr/ressources/pcg/classe-4/ ; line "Dettes fournisseurs et comptes rattachés" au passif: https://www.calebgestion.com/cours_comptabilite/c23_pcg_plan_comptable_general_suite.htm | High |
| 408 | Fournisseurs – Factures non parvenues | liability | credit | https://www.dougs.fr/ressources/pcg/classe-4/ | High |
| 411 | Clients (4111 Ventes de biens ou de prestations de services) | asset | debit | https://www.dougs.fr/ressources/pcg/classe-4/ | High |
| 418 | Clients – Produits non encore facturés (4181 Factures à établir) | asset | debit | https://www.dougs.fr/ressources/pcg/classe-4/ | High |
| 512 | Banques (5121 Comptes en euros) | asset | debit | https://www.dougs.fr/ressources/pcg/classe-5/ ; https://www.dougs.fr/ressources/pcg/classe-5/compte-512/ | High |
| 530 | Caisse | asset | debit | classe 53 « Caisse »: https://www.dougs.fr/ressources/pcg/classe-5/ ; « 512 – Banque, 53 – Caisse »: https://compta-cours.com/comptabilite/plan-comptable-general-et-plan-comptable-de-lentreprise/ | High |
| 164 | Emprunts auprès des établissements de crédit (useful for SME) | liability | credit | https://www.dougs.fr/ressources/pcg/classe-1/ | High |

Note: 108 « Compte de l'exploitant » exists for entreprises individuelles (https://www.dougs.fr/ressources/pcg/classe-1/).

## 2. VAT accounts (classe 445 – État, taxes sur le chiffre d'affaires)

| Code | Label FR | Type | Normal balance | Source | Confidence |
|---|---|---|---|---|---|
| 44551 | TVA à décaisser | liability | credit | https://www.l-expert-comptable.com/plan-comptable/compte-44551-tva-decaisser (describes it as the net of 44571 − 44566, credited when TVA collectée > TVA déductible); https://www.pennylane.com/fr/fiches-pratiques/plan-comptable/compte-44551-tva-a-decaisser | High |
| 44566 | TVA sur autres biens et services (input VAT on running purchases; task wording "TVA déductible sur autres biens et services") | asset | debit | https://www.pennylane.com/fr/fiches-pratiques/plan-comptable/compte-44566-tva-sur-autres-biens-et-services | Med-High |
| 44571 | TVA collectée (output VAT) | liability | credit | https://www.indy.fr/guide/tenue-comptable/plan-comptable/compte-classe-quatre/compte-44571/ | Med-High |

Also relevant: 44562 « TVA sur immobilisations » (input VAT on fixed assets) — https://www.mooncard.co/fr/cas-usage/tva/tva-deductible/tva-a-decaisser mentions 44562 for deductible VAT — **Med**. Credit-de-VAT counterpart account **44567 « Crédit de TVA à reporter »** — **Unverified** (the l-expert-comptable 44551 page alludes to "un autre compte spécifique" without naming it; do not hard-code without verification).

## 3. Revenue — classe 7 (produits)

| Code | Label FR | Type | Source | Confidence |
|---|---|---|---|---|
| 701 | Ventes de produits finis | income | https://www.dougs.fr/ressources/pcg/classe-7/ | High |
| 706 | Prestations de services | income | https://www.dougs.fr/ressources/pcg/classe-7/ | High |
| 707 | Ventes de marchandises | income | https://www.dougs.fr/ressources/pcg/classe-7/ | High |
| 708 | Produits des activités annexes | income | https://www.dougs.fr/ressources/pcg/classe-7/ | High |
| 709 | Rabais, remises et ristournes accordés (contra-revenue, 7096/7097/7098 mirror 706/707/708) | income (contra) | https://www.dougs.fr/ressources/pcg/classe-7/ | High |

## 4. Expenses — classe 6 (charges)

| Code | Label FR | Type | Source | Confidence |
|---|---|---|---|---|
| 601 | Achats stockés – Matières premières (et fournitures) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 606 | Achats non stockés de matières et fournitures | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 6061 | Fournitures non stockables (eau, énergie, etc.) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 607 | Achats de marchandises | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 613 | Locations (6132 Locations immobilières; task wording "613 loyers") | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 6226 | Honoraires (sous 622 Rémunérations d'intermédiaires et honoraires) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 624 | Transports de biens et transports collectifs du personnel | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 626 | Frais postaux et de télécommunications | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 631 | Impôts, taxes et versements assimilés sur rémunérations (6311 Taxe sur les salaires) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 633 | Impôts, taxes et versements assimilés sur rémunérations (autres organismes) (6331 Versement transport, 6333 contribution formation) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 635 | Autres impôts, taxes et versements assimilés (63512 Taxes foncières, 63514 TVS, 63511 CET) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 641 | Rémunérations du personnel (6411 Salaires, appointements) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 645 | Cotisations de sécurité sociale et de prévoyance (6451 URSSAF, 6452 mutuelles, 6453 retraites) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 661 | Charges d'intérêts (6611 Intérêts des emprunts et dettes) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 681 | Dotations aux amortissements, aux dépréciations et aux provisions (charges d'exploitation) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 6811 | Dotations aux amortissements sur immobilisations incorporelles (68112 … corporelles) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |
| 695 | Impôts sur les bénéfices (6954 Impôts dus à l'étranger) | expense | https://www.dougs.fr/ressources/pcg/classe-6/ | High |

## 5. Equity — classe 1 (capitaux)

| Code | Label FR | Source | Confidence |
|---|---|---|---|
| 101 | Capital social | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 104 | Primes liées au capital (1041 primes d'émission…) | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 1061 | Réserve légale | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 1068 | Autres réserves | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 110 | Report à nouveau – solde créditeur | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 119 | Report à nouveau – solde débiteur | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 120 | Résultat de l'exercice – bénéfice | https://www.dougs.fr/ressources/pcg/classe-1/ | High |
| 129 | Résultat de l'exercice – perte | https://www.dougs.fr/ressources/pcg/classe-1/ | High |

Affectation rule (commercial code): 5% of profit to réserve légale until it reaches 10% of capital (art. L. 232-10 code de commerce) — **Med-High, general knowledge, not re-verified this pass** (legifrance URL not fetched).

## 6. Amortissements (contra-assets + dotations)

- Contra-asset accounts (bilan, negative asset): **281 Amortissements des immobilisations corporelles** (subs 2811 incorporelles…, 2815 installations, 2818 autres), **280 Amortissements des immobilisations incorporelles**, **282 Amortissements des immobilisations mises en concession**. Source: https://www.dougs.fr/ressources/pcg/classe-2/ — High. (Dépréciations in 29x — same page.)
- Charge side (compte de résultat): **6811 Dotations aux amortissements sur immobilisations incorporelles** (68112 corporelles), umbrella 681; financial side 686. Source: https://www.dougs.fr/ressources/pcg/classe-6/ — High.

## 7. TVA settlement (44551)

Mechanism (verified): TVA à décaisser = TVA collectée (44571) − TVA déductible (44566/44562); 44551 is credited with the net due; payment entry: Dr 44551 / Cr 512. When TVA déductible > collectée the balance is a credit de TVA (see §2, 44567 unverified). Source: https://www.l-expert-comptable.com/plan-comptable/compte-44551-tva-decaisser — High.

## 8. Year-end (clôture)

- Result of the year accumulates in **120 (bénéfice) / 129 (perte)**. At affectation, 120 → 1061 (réserve légale), 1068 (autres réserves), 457 (dividendes à payer), remainder → 110 (report à nouveau créditeur) / 119 (débiteur). Sources: dougs classe-1 (account existence) + compta-facile classe-1 description (« le bénéfice de l'exercice (compte 120) ou la perte (compte 129), les reports à nouveau (comptes 11), les réserves (comptes 106) ») — https://www.compta-facile.com/plan-des-comptes-en-comptabilite/ — High for accounts, Med-High for affectation flow (standard practice).
- Suggest for fr.js: `closing: { resultAccount: '120', equityAccount: '110' }` (mirror LU pattern).

## 9. Statutory class mapping (bilan / compte de résultat) — SME (plan comptable abrégé)

Note on "abrégé": under the 2022-06 reform (applicable 2025+), the **chart of accounts is unique** (système abrégé/développé distinction abolished); a *simplified presentation* of annual accounts still exists for small/micro entities as a filing simplification, but the account plan is common. Sources: https://www.compta-online.com/plan-comptable-general-pdf-ao2428 (High); ANC PDF (High); https://www.plancomptable.com/comptes-annuels/etablissement/presentation-des-comptes-annuels/systeme-de-base-et-simplifiee/ (Med).

Bilan — ACTIF (classes 1–5):
- Immobilisations (actif non courant): classe 2 (20/21/23/26/27 brut; 28 amortissements & 29 dépréciations en diminution — actif net = brut − amortissements − dépréciations). Source: compta-facile classe-2 — High.
- Stocks et en-cours: classe 3 (31 matières, 32 autres approvisionnements, 33/34 en-cours, 37 marchandises, 39 dépréciations). Source: compta-facile classe-3 — High.
- Créances (actif circulant): classe 4 (411 clients, 409 fournisseurs débiteurs, 44 État, 445 TVA déductible, 486 CCA…). Source: compta-facile classe-4 + calebgestion — High.
- Trésorerie (disponibilités): classe 5 (512 banques, 530 caisse, 50 VMP). Source: compta-facile classe-5 — High.
- Depuis 2026: nouvelle ligne « autres fonds propres » au passif (règlement 2024-07). Source: compta-online — High.

Bilan — PASSIF:
- Capitaux propres: classe 1 hors 16 (101 capital, 104 primes, 106 réserves, 11 report à nouveau, 12 résultat, 13 subventions d'investissement, 14 provisions réglementées). Source: compta-facile classe-1 — High.
- Autres fonds propres (new 2026 line): 16x emprunts assimilés / titres participatifs, avances conditionnées, droits du concédant. Source: compta-online — High.
- Provisions pour risques et charges: 15. Source: compta-facile — High.
- Dettes: 16 (emprunts), 40 (fournisseurs 401/408), 42 (personnel), 43 (organismes sociaux), 44 (État: 444 IS, 445 TVA collectée, 447…), 45 (associés comptes courants), 46 (débiteurs/créditeurs divers), 487 PCA. Source: compta-facile classe-4/1 — High.

Compte de résultat (classe 6 charges / classe 7 produits):
- Chiffre d'affaires: 70 (701/706/707/708 − 709). Produits d'exploitation: 70–75. Charges d'exploitation: 60–65. Charges financières: 66; produits financiers: 76. Dotations: 68; reprises: 78. Participation + IS: 69 (691 participation, 695 IS). Résultat exceptionnel (67/77) désormais réservé aux événements majeurs et inhabituels (2022-06). Sources: compta-facile classes 6/7 (High) + compta-online reform notes (High).

## 10. TVA rates 2026 + regimes (verified on government sources)

Rates (France métropolitaine, unchanged for 2026):
- **20 % taux normal** (default) — https://entreprendre.service-public.gouv.fr/vosdroits/F32231 (snippet: « Taux normal de TVA : 20% (France métropolitaine). Code général des impôts : article 296 »), https://entreprendre.service-public.gouv.fr/vosdroits/F22399 — High.
- **10 % taux intermédiaire** — https://entreprendre.service-public.gouv.fr/vosdroits/F22399 — High.
- **5,5 % taux réduit** — https://entreprendre.service-public.gouv.fr/vosdroits/F22399 , https://entreprendre.service-public.gouv.fr/vosdroits/F31596 — High.
- **2,1 % taux particulier** (presse, médicaments remboursables…) — https://www.economie.gouv.fr/particuliers/impots-et-fiscalite/gerer-mes-autres-impots-et-taxes/tva-quels-sont-les-taux-de-votre-quotidien (page itself failed to load twice; snippet « un taux normal (20 %), deux taux réduits (10 % et 5,5 %) et un taux particulier (2,1 %) ») + https://www.l-expert-comptable.com/a/529638-les-differents-taux-de-tva-en-france.html (2026 article) — Med-High.

Regimes (https://www.impots.gouv.fr/professionnel/les-regimes-dimposition-la-tva, modified 21/05/2026 — High; https://entreprendre.service-public.gouv.fr/vosdroits/F21746, vérifié 01/01/2026 — High):
- **Franchise en base** (no VAT charged/declared; no input VAT deduction; invoice mention « TVA non applicable, art. 293 B du CGI » / art. L. 223-3 CIBS): seuil de base 85 000 € (ventes de marchandises/logement) / 37 500 € (prestations de services); seuils majorés 93 500 € / 41 250 € (dépassement → TVA dès le 1er jour de dépassement). Applicable from 1 Jan 2025, unchanged for 2026 — the LF2025 "unique 25 000 € threshold" was **abandoned** (law of 3 Nov 2025). Also: activités libérales 37 500/41 250.
- **Régime réel simplifié (RSI)**: CA between 85 000–840 000 € (ventes/hébergement) or 37 500–254 000 € (services), TVA due < 15 000 €; 2 acomptes semestriels (July 55 %, Dec 40 % of prior-year VAT) + déclaration annuelle de régularisation CA12 (n° 3517-S) due by the 2nd working day after 1 May.
- **Régime réel normal**: CA > 840 000 € (ventes/hébergement) or > 254 000 € (services); CA3 monthly (or quarterly under 4 000 € annual VAT), plus annual CA12.
- Flag: **recodification CIBS** — ordonnance n° 2025-1247 (17 Dec 2025) moves VAT provisions from the CGI to the Code des impositions des biens et des services from **1 Sep 2026** (article references in profiles should expect CGI→CIBS shift; impots.gouv already cites both).

## Bonus: identifiers (useful for fr.js)

- TVA intracommunautaire number: **FR + 2-digit key + 9-digit SIREN** — https://entreprendre.service-public.gouv.fr/vosdroits/F23570 — High. Regex suggestion: `/^FR\d{11}$/i`.
- Company id: SIREN (9 digits) / SIRET (14). Not re-verified this pass — Med (general knowledge).

---

## Flags / unverifiable items

1. **44567 « Crédit de TVA à reporter »** (credit-de-VAT counterpart to 44551) — Unverified this pass; commonly used in practice; verify against a PCG list before hard-coding.
2. **Affectation 5 % → réserve légale, plafond 10 % du capital** (art. L. 232-10 code de commerce) — Med-High general knowledge; legifrance URL not fetched.
3. **2,1 % rate** — official economie.gouv page failed to load twice; confirmed only via search snippet + commercial 2026 article. Med-High.
4. **Compte 205 label**: dougs lists « …logiciels… » with 2056 Logiciels, but règlement ANC 2023-05 renamed the heading to « solutions informatiques » (2053). Cosmetic; use the 2023-05 wording for new charts. Med-High (compta-online).
5. **Peppol scheme for FR** (0208 SIREN) and SIRET regex — Unverified this pass.
6. **"613 loyers"** — the PCG label is « Locations » (6132 Locations immobilières); task wording "loyers" is colloquial.
7. Bilan abrégé *filing* thresholds for micro-entreprises (comptes annuels simplifiés) — not researched this pass (out of scope for the chart itself; chart is unique since 2022-06).
8. economie.gouv.fr and lecoindesentrepreneurs.fr pages returned http_error on extraction — rate/PCG claims from those URLs rest on snippets only.

## Key sources (URL index)

- https://entreprendre.service-public.gouv.fr/vosdroits/F21746 — franchise en base (vérifié 01/01/2026)
- https://www.impots.gouv.fr/professionnel/les-regimes-dimposition-la-tva — TVA regimes (21/05/2026)
- https://entreprendre.service-public.gouv.fr/vosdroits/F23570 — numéro TVA intracommunautaire
- https://entreprendre.service-public.gouv.fr/vosdroits/F32231 , F22399 , F31596 — TVA rates
- https://www.compta-online.com/plan-comptable-general-pdf-ao2428 — PCG 2026 reform overview (règlements 2022-06 / 2024-07 / 2026-01-02)
- https://www.anc.gouv.fr/files/anc/files/1_Normes_fran%C3%A7aises/Reglements/Recueils/PCG_Janvier2024/PCG--1er-janvier-2024.pdf — official consolidated PCG PDF
- https://www.dougs.fr/ressources/pcg/ (+ /classe-1/ /classe-2/ /classe-4/ /classe-5/ /classe-6/ /classe-7/) — full account list with labels
- https://www.l-expert-comptable.com/plan-comptable (+ /compte-44551-tva-decaisser) — class index + 44551 mechanics
- https://www.compta-facile.com/plan-des-comptes-en-comptabilite/ — class-by-class bilan/P&L mapping
- https://www.pennylane.com/fr/fiches-pratiques/plan-comptable/compte-44566-tva-sur-autres-biens-et-services ; https://www.indy.fr/guide/tenue-comptable/plan-comptable/compte-classe-quatre/compte-44571/ — VAT account labels
