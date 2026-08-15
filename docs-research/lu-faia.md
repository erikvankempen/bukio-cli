# Luxembourg FAIA — Fichier d'Audit Informatisé AED — Research Brief

**Purpose:** support the bukio-cli Luxembourg profile (`src/jurisdictions/lu.js`, PCN 2020) FAIA audit-file export (B3).
**Current version:** FAIA **2.01** (final version AED 31-01-2013). Still the version published on the official portal (page last updated 21/07/2020). No newer version found (search 2024–2026 sources still reference 2.01). [high]
**Variant to target for bukio-cli:** **Reduced version B** — bookkeeping without invoicing module (see §2).
**Local copies of all primary sources** were downloaded to `/root/bukio-cli/docs-research/faia-src/` (XSD zip, FAIA_2_01.pdf, FAIA-recommandation.pdf, FAIA-FAQ.pdf) — inspect the XSD directly; do not guess.

---

## 1. Legal basis [high]

- **Loi du 19 décembre 2008 concernant la coopération interadministrative entre les administrations fiscales** (Mémorial A-206 du 24 décembre 2008), amending **article 70, paragraphe 3, 2e alinéa LTVA** (Loi modifiée du 12 février 1979 concernant la TVA): when books/documents/data exist electronically, they must on demand by the administration be communicated in a legible, directly intelligible form, on paper **"ou suivant toutes autres modalités techniques que l'administration détermine"** — i.e. the electronic format is determined by the administration.
  - Source: https://pfi.public.lu/fr/professionnel/tva/faia/faia-201.html (quotes the law verbatim) [high]
  - Law text: http://www.legilux.public.lu/leg/a/archives/2008/0206/a206.pdf [high]
- **Who is bound:** "tout assujetti qui dispose d'un système de comptabilité informatique est tenu, sous peine de sanctions, de délivrer des données par la voie électronique, dans les cas où cette demande est exprimée par l'administration" — i.e. any VAT-registered taxpayer with **computerised accounting**, **on demand during a VAT audit** (FAIA is *not* filed with the VAT return; on-demand only). Sources: pfi.public.lu FAIA page [high]; FAQ Q"Est-ce que le FAIA est à remettre systématiquement?" → "Non, le FAIA est seulement à remettre sur demande par un des agents de l'AED" [high]; https://www2.deloitte.com/lu/en/pages/tax/solutions/faia.html [medium].
- **Not bound (exemptions, official FAQ "Aspects généraux"):** the obligation does NOT apply to: (a) VAT taxpayers **not subject to the PCN** (plan comptable normalisé); (b) taxpayers benefiting from the **simplified VAT regime** (régime simplifié); (c) taxpayers whose **annual turnover ≤ €112,000.00**; (d) taxpayers whose volume of accounting transactions stays within "reasonable limits" (**±500 transactions**) where manual control is more rational than an electronic export. A turnover > €112k *with* ≤ 500 transactions is still exempt; PCN-subject is a *sine qua non* (turnover > €112k and > 500 transactions without PCN → not bound, though AED can demand structured data under art. 70 anyway).
  - Source: https://pfi.public.lu/dam-assets/backup/FAIA/FAIA/FAIA-FAQ.pdf (FAIA-FAQ.pdf, mars 2013, pages 1–2) [high]
  - Corroborated by https://saft-validator.com/blog/what-is-faia-complete-guide ("annual turnover exceeding €112,000") [medium]
- **Since when:** legal basis 19 December 2008; first AED FAIA recommendations published **end 2009** (FAIA 1.0); revised **FAIA 2.0 in October 2011**; **FAIA 2.01 in February 2013** (FAQ doc dated mars 2013). Only the latest recommendation is valid (FAQ: "la dernière recommandation qui a été publiée est celle qui est en vigueur"). Sources: https://de.wikipedia.org/wiki/Fichier_d%E2%80%99Audit_Informatis%C3%A9_AED (citing PwC LU) [medium]; FAQ Q"1re recommandation FAIA 1.0 vs 2.0" [high].
- **Note / flag:** FAIA is an administrative **recommandation** (technical modality) under art. 70 LTVA — there is no separate law named "FAIA". The €112k/500-transaction/PCN carve-outs are AED practice from the FAQ, not statute text. The PCN obligation itself comes from Luxembourg accounting law (Code de commerce) — not verified in this pass. [flag]

## 2. File format, variants, encoding [high unless noted]

- **XML** — "Le format FAIA est l'unique format qui est accepté par l'AED pour l'exportation des exercices comptables" (recommandation §3). Source: https://pfi.public.lu/dam-assets/backup/FAIA/FAIA/FAIA-recommandation.pdf [high]
- **XSD schemas** (official, downloadable):
  - ZIP of all schemas: `https://pfi.public.lu/content/dam/pfi/backup/FAIA/FAIA/XSD_Files.zip` (~13 MB; note: the `dam-assets` mirror returns an empty 200 — use `/content/dam/` path). Contains `FAIA_v_2.01_full.xsd`, `FAIA_v_2.01_reduced_version_A.xsd`, `FAIA_v_2.01_reduced_version_B.xsd` + PDF descriptions. Source: https://pfi.public.lu/fr/professionnel/tva/faia/faia-201.html ("All original XSD Schemas packed in a ZIP file") [high]
  - Schema location declared in the official description doc: `http://www.aed.public.lu/FAIA/FAIA/FAIA_v_2_01_full.xsd` (historical URL; pfi.public.lu is the live home). Source: FAIA_2_01.pdf p.1 [high]
- **Three variants** (recommandation §2 + FAQ "Aspects comptables"):
  - **Full** (`FAIA_2.01_v1_full` / `FAIA_v_2.01_full.xsd`): integrated systems covering accounting + invoicing + stock/assets etc.
  - **Reduced A** (`FAIA_v2.01_reduced_version_A`): accounting + invoicing modules (adds `SourceDocuments`).
  - **Reduced B** (`FAIA_2.01_reduced_version_B`): non-integrated / accounting-only. FAQ: "Si vous ne disposez pas de logiciel de facturation, vous pouvez utiliser le schéma FAIA_20.1_reduced_version_B." → **bukio-cli should target reduced B** (no SourceDocuments element in that schema — verified). [high]
- **Encoding:** no explicit encoding requirement found in AED docs. XML default **UTF-8** is used in practice (XSDs carry `<?xml version="1.0"?>` with no encoding attr). [medium — not stated by AED]
- **Namespace (implementation-critical):** Full XSD: `targetNamespace="urn:OECD:StandardAuditFile-Taxation/2.00"`, schema version="2.00", id="FAIA-T". **Reduced A and B: NO targetNamespace** (namespace-less), schema version="2.01", id="FAIA" (verified by direct inspection of the XSD files). [high]
- **Filename convention:** **none found** in the official recommendation/FAQ/description docs — no mandated filename. [flag — not verifiable; third-party tools use e.g. `FAIA_<year>.xml`]
- **Media/delivery:** no file size limit; standard media (CD-R, DVD-R, memory stick, HDD externe, e-mail); sensitive data must be delivered encrypted with the decryption key provided separately (FAQ "Aspects techniques"). [high]
- **Date format:** ISO 8601 `YYYY-MM-DD` (`xs:date` in XSD). [high]

## 3. Top-level structure [high — from XSD]

```
<AuditFile>                          ← root element (required)
├── <Header>                         ← required
│     AuditFileVersion, AuditFileCountry ("LU"), AuditFileRegion (opt),
│     AuditFileDateCreated, SoftwareCompanyName, SoftwareID, SoftwareVersion,
│     Company (CompanyHeaderStructure), DefaultCurrencyCode (EUR),
│     SelectionCriteria (opt), HeaderComment (opt),
│     + FAIA extension: TaxAccountingBasis (required), TaxEntity (opt)
├── <MasterFiles>                    ← opt
│     GeneralLedgerAccounts, Taxonomies (must NOT be populated), TaxTable,
│     Customers, Suppliers, UnitsOfMeasure, AnalysisTypes, Movements,
│     Products, PhysicalStock, Assets, Custom, ...
├── <GeneralLedgerEntries>           ← opt
│     NumberOfEntries, TotalDebit, TotalCredit, Journal*
└── <SourceDocuments>                ← opt; full + reduced A only
      SalesInvoices, PurchaseInvoices, Payments, MovementOfGoods, ...
```

- **Header block** (`HeaderStructure`, XSD lines ~2595–2656): `AuditFileVersion` ("2.01"), `AuditFileCountry` = `LU` (ISO 3166-1), `AuditFileDateCreated`, `SoftwareCompanyName`/`SoftwareID`/`SoftwareVersion`, `Company`, `DefaultCurrencyCode` (ISO 4217, EUR), `SelectionCriteria`, `HeaderComment`. FAIA-specific: **`TaxAccountingBasis`** (required: "Invoice Accounting, Cash Accounting, Delivery, other …") and `TaxEntity` (opt). Source: https://pfi.public.lu/fr/professionnel/tva/faia/faia-201/faia-version-2-full.html + XSD [high]
- **Company identification** (`CompanyHeaderStructure`): `RegistrationNumber` (required — RCS / company registration number), `Name`, `Address` (AddressStructure: StreetName, BuildingNumber, City, PostalCode, Country, Region…), `Contact` (required, with ContactPerson + Telephone), `TaxRegistration` (opt, repeated), `BankAccount` (opt). [high]
- **Tax identification** (`TaxIDStructure`, XSD ~2758–2788): `TaxRegistrationNumber` — "Unique number issued by AED for use by the company. (LU123456-78)" → this is the **matricule** (AED national number, format LU123456-78); `TaxType` (opt); **`TaxNumber`** (opt — "The tax registration number for the particular tax regime referred to by TaxType, f.i. VAT-number" → the **TVA number**); `TaxAuthority` (opt); `TaxVerificationDate` (opt). [high]
  - Interpretation: RCS → `Company/RegistrationNumber`; matricule → `Company/TaxRegistration/TaxRegistrationNumber`; TVA number → `Company/TaxRegistration/TaxNumber`. [medium — field semantics from XSD docs]
- **SelectionCriteria** (required in practice): the FAIA must cover a **complete civil year** (01/01–31/12); truncated periods are **not permitted**; one file per period ("chaque fichier FAIA ne devra inclure qu'une seule période"). Use `SelectionStartDate`/`SelectionEndDate` (or PeriodStart/PeriodStartYear/PeriodEnd/PeriodEndYear). Sources: XSD annotation ("It's not possible to submit a FAIA with a period that is not matching the civil year") [high]; FAQ [high].
- **AccountType values are French:** "Type of account - Asset/Liability/Sale/Expense **Actif/Passif/Produit/Charge**" (XSD annotation). Using English labels is a common rejection cause (saft-validator.com) [medium]. Valid FAIA requires these four values only. [high]

## 4. Chart of accounts section (PCN) [high — XSD]

`MasterFiles > GeneralLedgerAccounts > Account` (unbounded), per account:
- `AccountID` (required) — GL account code, may include sub-account levels.
- `AccountDescription` (required) — account label.
- `StandardAccountID` (opt) — "AccountID based on a standard prescribed by authorities. Holds the Standard Chart of Accounts Numbering. **StandardAccountID must be populated if the element AccountID is not the same.**" → for PCN 2020: put the **PCN code here** when the internal code differs. [high]
- `GroupingCategory`, `GroupingCode` (opt) — grouping for financial statement reconciliation.
- `AccountType` (required) — one of **Actif / Passif / Produit / Charge**.
- `AccountCreationDate` (opt, xs:date).
- Opening balance: choice `OpeningDebitBalance` | `OpeningCreditBalance`; closing: choice `ClosingDebitBalance` | `ClosingCreditBalance` (FAIAmonetaryType, header currency).
- `xs:any` extension point (opt).

**PCN 2020 mapping** (inference for bukio-cli, verify against lu.js chart): PCN classes 1–7 → AccountType: classes **1 Capital / 2 Immobilisations / 3 Stocks / 4 Tiers / 5 Comptes financiers** → Actif or Passif depending on normal balance; class **6 Charges** → Charge; class **7 Produits** → Produit. The XSD does not encode classes — only the four types. [low — my inference, not in any source]
- The FAIA docs themselves never mention PCN numbers; PCN-subject status is only an eligibility criterion (FAQ). [high]

## 5. Journal / transactions section [high — XSD]

`GeneralLedgerEntries`: `NumberOfEntries`, `TotalDebit`, `TotalCredit`, then `Journal`* (unbounded):
- **Journal**: `JournalID` (required — "Source GL journal identifier"), `Description` (required), `Type` (required — "Grouping mechanism for journals", e.g. OD/VTE/VAC/BQ style codes; free text code in schema).
- **Transaction** (unbounded): `TransactionID` (required — cross-reference to GL posting / entry number), `Period` (required, accounting period 1–12), `PeriodYear` (required, 1970–2100), `TransactionDate` (required, xs:date — document date), `SourceID` (opt — person/application that entered), `TransactionType` (opt — "normal, (automated) periodically, etc."), `Description` (required — description of the journal transaction), `BatchID` (opt), `SystemEntryDate` (required — date captured by system), `GLPostingDate` (required — date posted to GL), `CustomerID` (opt), `SupplierID` (opt), `SystemID` (opt — system document number).
- **Line** (unbounded): `RecordID` (required — "Identifier to trace entry to journal line or posting reference"), `AccountID` (required), `Analysis` (opt, unbounded — AnalysisType/AnalysisID cost centres), `ValueDate` (opt), `SourceDocumentID` (opt — source document number), `CustomerID` / `SupplierID` (opt — counterparty refs at line level), `Description` (required — line description), **choice `DebitAmount` | `CreditAmount`** (required — see §6), `TaxInformation` (opt, unbounded — see §7).
- **Counterparty/partner info** lives in `MasterFiles > Customers` / `Suppliers`: `CustomerID` (unique; generic **"FINAL CONSUMER"** record required for final consumers — XSD annotation), `Name`, `Address`, `SelfBillingIndicator`, `AccountID` (GL account for the customer), opening/closing balances; Supplier identical shape. Transactions reference these via `CustomerID`/`SupplierID` (FAQ: these are key/reference fields that must be present). [high]
- **Distinct dates** (FAQ "Aspects comptables"): TransactionDate = document date (encoded manually), SystemEntryDate = auto-generated capture date, GLPostingDate = posting date. [high]

## 6. Debit/credit vs signed balances [high — XSD]

- **Separate debit/credit amounts, NOT signed balances.** Every `Line` carries exactly one of `<DebitAmount>` or `<CreditAmount>` (xs:choice), each an `AmountStructure` = `<Amount>` (FAIAmonetaryType, header currency) + optional `<CurrencyCode>`/`<CurrencyAmount>` (foreign currency).
- Account master balances likewise use paired choices `OpeningDebitBalance`|`OpeningCreditBalance`, `ClosingDebitBalance`|`ClosingCreditBalance` — never a signed number.
- Totals are reported as `TotalDebit` + `TotalCredit` (GeneralLedgerEntries and each SourceDocuments group).

## 7. VAT/tax section [high — XSD]

- **`MasterFiles > TaxTable > TaxTableEntry`** (unbounded): `TaxType` (fixed value **"TVA"**), `Description` (fixed **"Taxe sur la valeur ajoutée"**), then `TaxCodeDetails` (unbounded): `TaxCode` (required, application-specific code), `EffectiveDate`/`ExpirationDate` (opt), `Description` (opt), choice `TaxPercentage` | `FlatTaxRate`, `Country` (required, ISO 3166-1), `Region` (opt).
  - FAQ: tax codes must be specific per type/rate — e.g. `IAM15` for import of goods at 15% ("Import (achat) marchandise à 15% = IAM15"); distinct codes for domestic, intra-EU acquisitions, third-country imports, per rate. [high]
  - Current LU VAT rates (not in the XSD; rates evolve): 17% standard, 14% intermediate, 8% reduced, 3% super-reduced (as of 2023+). Source: https://saft-validator.com/blog/what-is-faia-complete-guide [medium]. The FAQ's 15% example reflects the 2013 rate.
- **Per-posting-line VAT:** `Line > TaxInformation` (TaxInformationStructure): `TaxType` (opt), `TaxCode` (opt), `TaxPercentage` (opt), `TaxBase` (opt), `TaxBaseDescription` (opt), `TaxAmount` (AmountStructure), `TaxExemptionReason` (opt), `TaxDeclarationPeriod` (opt). [high]
- **No VAT-return-style section:** FAIA carries VAT via the TaxTable + per-line TaxInformation; it is an audit file, not a VAT return (FAQ: not filed with the declaration). [high]

## 8. Element naming [high — XSD]

- **Element names are English / OECD SAF-T 2.0**, not French: `<AuditFile>`, `<Header>`, `<Company>`, `<TaxRegistration>`, `<MasterFiles>`, `<GeneralLedgerAccounts>`, `<Account>`, `<AccountID>`, `<AccountDescription>`, `<AccountType>`, `<TaxTable>`, `<GeneralLedgerEntries>`, `<Journal>`, `<Transaction>`, `<Line>`, `<DebitAmount>`, `<CreditAmount>`.
- **No** `<Ecriture>`, `<Piece>`, `<MontantDebit>`, `<MontantCredit>` — those are other francophone SAF-T variants (e.g. French FEC/SAF-T, Dutch XAF uses similar English naming); FAIA 2.01 is a direct OECD SAF-T 2.0 derivative (XSD header: "OECD Standard Audit File", edited by the AED). [high]
- **Content values are French** where the schema prescribes them: AccountType = Actif/Passif/Produit/Charge; TaxType = TVA; "Taxe sur la valeur ajoutée"; "FINAL CONSUMER" customer designation. [high]

## 9. Confidence summary & unverifiable items

| # | Item | Confidence | Source |
|---|------|-----------|--------|
| 1 | Legal basis: loi du 19-12-2008 + art. 70(3) al. 2 LTVA; on-demand, sanctions | high | pfi.public.lu; Mémorial A-206 |
| 1 | Exemptions: non-PCN, régime simplifié, CA ≤ €112,000, ≤ ±500 transactions | high | FAIA-FAQ.pdf |
| 1 | Timeline 2009/2011/2013; 2.01 current | high (2.01)/medium (dates) | pfi.public.lu; Wikipedia (PwC) |
| 2 | XML only; 3 XSD variants; reduced B for accounting-only | high | recommandation + FAQ + XSD |
| 2 | XSD URL (content/dam path) | high | pfi.public.lu (downloaded) |
| 2 | Namespace: full=urn:OECD…/2.00; reduced A/B=none | high | XSD inspection |
| 2 | Encoding UTF-8 | medium (not stated by AED) | — |
| 2 | Filename convention | **not verifiable** — none specified | — |
| 3 | Root AuditFile; Header/MasterFiles/GeneralLedgerEntries/SourceDocuments | high | XSD |
| 3 | Company/RCS, matricule (TaxRegistrationNumber LU…-…), TVA (TaxNumber) | high | XSD |
| 4 | Account structure incl. StandardAccountID for PCN code | high | XSD |
| 4 | PCN class→AccountType mapping | low (inference) | — |
| 5 | Journal/Transaction/Line structure | high | XSD |
| 6 | DebitAmount/CreditAmount, not signed | high | XSD |
| 7 | TaxTable (TVA) + line-level TaxInformation | high | XSD |
| 7 | Current rates 17/14/8/3 | medium | saft-validator.com (secondary) |
| 8 | English element names, French content values | high | XSD |

**Explicitly not verifiable / flags:**
- No official filename convention for FAIA files (AED docs silent).
- No explicit encoding statement (UTF-8 assumed; safe for French accents).
- PCN 2020 class→AccountType mapping not documented anywhere — derive from lu.js chart normal balances.
- The €112k threshold is AED FAQ practice, not statute; "régime simplifié" boundaries (annual declaration) not researched here.
- Whether AED will ever release a version > 2.01: no evidence; portal unchanged since 2020.
- No official AED validator tool exists (FAQ: "Seul le schéma publié au site Internet de l'AED peut servir de mécanisme de contrôle") — validate against the XSD.

---

### Primary source URLs
- FAIA 2.01 index (legal basis, links): https://pfi.public.lu/fr/professionnel/tva/faia/faia-201.html
- Element descriptions (full version): https://pfi.public.lu/fr/professionnel/tva/faia/faia-201/faia-version-2-full.html
- Full description doc PDF: https://pfi.public.lu/dam-assets/backup/FAIA/FAIA/FAIA_2_01.pdf (133 pp, "FAIA 2.01 full version" — the detailed field-by-field spec)
- Recommendation PDF (mars 2013): https://pfi.public.lu/dam-assets/backup/FAIA/FAIA/FAIA-recommandation.pdf
- FAQ PDF (mars 2013): https://pfi.public.lu/dam-assets/backup/FAIA/FAIA/FAIA-FAQ.pdf
- XSD ZIP (working URL): https://pfi.public.lu/content/dam/pfi/backup/FAIA/FAIA/XSD_Files.zip
- Mémorial A-206 (loi 19-12-2008): http://www.legilux.public.lu/leg/a/archives/2008/0206/a206.pdf

### Secondary sources
- https://saft-validator.com/blog/what-is-faia-complete-guide (threshold, variants, common errors, current rates)
- https://www2.deloitte.com/lu/en/pages/tax/solutions/faia.html (SAF-T/FAIA on-demand confirmation)
- https://de.wikipedia.org/wiki/Fichier_d%E2%80%99Audit_Informatis%C3%A9_AED (version timeline, cites PwC)
- https://www.odoo.com/documentation/14.0/fr/applications/finance/fiscal_localizations/luxembourg.html (Odoo LU localization — FAIA generator reference implementation)
