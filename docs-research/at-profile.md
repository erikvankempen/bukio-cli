# Austria — bukio jurisdiction profile (AT)

Phase C profile. Research verified 15 August 2026. Every item source-verified;
confidence per item.

## 1. Tax system

VAT (Umsatzsteuer) — EU member, SEPA.

| Item | Value | Source |
|---|---|---|
| Standard rate | **20 %** | UStG § 10; VATupdate 2026 guide |
| Intermediate rate | **13 %** (Zwischensteuersatz — catering, cultural events, tourism, accommodation) | VATupdate 2026; TaxRavens 2026 |
| Reduced rate | **10 %** (food, books, medicines, residential rent, public transport) | VATupdate 2026; TaxRavens 2026 |
| New rates 2026 | 4.9 % for selected basic food from **1 Jul 2026**; 0 % on contraceptives/feminine hygiene from 1 Jan 2026 (not standing codes — noted) | taxenlight 2026; TaxRavens 2026 |
| Kleinunternehmer | **€55,000** annual turnover since 1 Jan 2025 (was €35,000) — no VAT charged, § 6/1 Z 27 UStG note on invoices, exempt from UVA | VATupdate 2026; Fiscal Solutions 2026 |
| Reverse charge | § 19 Abs. 1 UStG (EU) — Leistungsempfänger als Steuerschuldner; Bauleistungen § 19 Abs. 1a | BMF |

Confidence: high (multiple 2026 sources agree on 20/13/10 and the €55K threshold).

## 2. VAT returns (UVA)

| Item | Value | Source |
|---|---|---|
| Return | **Umsatzsteuervoranmeldung (UVA)** + annual Umsatzsteuererklärung | BMF/usp.gv.at |
| Frequency | **Quarterly** when prior-year turnover ≤ €100,000; **monthly** above | Eurofiscalis 2026 |
| Deadline | **15th of the SECOND following month** (both monthly and quarterly: Jan → 15 Mar; Q1 → 15 May) | usp.gv.at; Taxenlight 2026; VATupdate 2026 |
| Annual return | **30 June** of the following year (electronic filing via FinanzOnline is mandatory; 30 Apr is the paper deadline) | Marosa VAT manual; Taxenlight 2026 |
| Registration | From €55,000 turnover | Fiscal Solutions 2026 |

Confidence: high.

## 3. Identifiers

| Item | Value | Source |
|---|---|---|
| VAT number (UID) | `ATU` + **8 digits** (ATU12345678) | Taxenlight 2026; Docuflair; Microsoft entity defs |
| Company register | **Firmenbuchnummer (FN)** — free text (e.g. FN 123456a) | BMF/Firmenbuch |
| Peppol EAS | **9914** — Österreichische Umsatzsteuer-Identifikationsnummer | Peppol BIS 3.0 EAS codelist |
| Bank | IBAN, SEPA core | — |

Confidence: high.

## 4. Chart of accounts

**Einheitskontenrahmen (EKR)** — the dominant Austrian SME convention and the
official BMF SAF-T chart (`EKR_fuer_SAF-T.csv`). Ten classes (0 Anlagevermögen,
1 Vorräte, 2 Forderungen/USt/Kassa/Bank, 3 Verbindlichkeiten, 4 Erlöse,
5 Wareneinsatz, 6 Personal, 7 sonstige Aufwendungen, 8 Steuern, 9 Eigenkapital/
Abschluss). BMF SAF-T uses short 3-digit codes; bukio requires 4-digit codes →
**zero-padded** (0620 Büromaschinen, 0630 PKW, 0660 BGA, 0689 kum. AfA).

Key accounts (BMF SAF-T chart): 2000 Forderungen L+L Inland, 2500 **Vorsteuer**,
2700 Kassa, 2800 Guthaben bei Kreditinstituten, 3110 Verbindlichkeiten ggü.
Kreditinstituten, 3300 Lieferverbindlichkeiten Inland, 3500 **Umsatzsteuer**,
3600 soziale Sicherheit, 3700 übrige sonstige Verbindlichkeiten, 4000 Erlöse
20 %, 4010 Erlöse 10 %, 4800 übrige betriebliche Erträge, 5000 Wareneinsatz,
6000 Löhne, 7200 Instandhaltung, 7400 Mietaufwand, 7600 Büromaterial, 7700
Versicherungen, 8500 Körperschaftsteuer, 9010 Stammkapital, 9350
Jahresgewinn/-verlust, 9380 Gewinnvortrag.

Note: the EKR has **single** VAT accounts (2500 Vorsteuer / 3500 Umsatzsteuer) —
no per-rate split like SKR 03's 1771/1776.

Confidence: high (primary source: BMF SAF-T CSV).

## 5. Legal forms & fiscal year

- Forms: GmbH, AG, OG, KG, e.U. (eingetragenes Einzelunternehmen)
- Fiscal year: calendar year default (12-31); any 12-month period allowed
- Annual accounts: no public filing for small GmbH (Offenlegung only above the
  § 221 UGB size thresholds) — not calendarised
- Corporate tax: 23 % Körperschaftsteuer (2026)

Confidence: high.

## 6. E-invoicing / Peppol

- Peppol participant; **B2G e-invoicing mandatory since 2014** (e-Rechnung.gv.at)
- B2B voluntary; EU B2B mandate planned from 2027/2028
- Peppol BIS 3.0 UBL accepted (scheme 9914 = UID)

Confidence: high.

## 7. B-milestones (not registered — strict dispatch fails loudly)

- `tax.returnLayout` — UVA return engine (B-milestone)
- `reporting.format` — UGB Jahresabschluss (B-milestone)
- `documents.auditFile` — **SAF-T AT** exists but is an OECD-style XML, NOT the
  Dutch Auditfile Financieel 4.0 the XAF builder emits (B-milestone)
- `documents.invoiceCompliance` — **registered: the art. 226 EU baseline**
  ('eu-invoice-vereisten'); § 11 UStG additions are a B-milestone

## 8. at.js mapping

| bukio field | Value |
|---|---|
| meta.country / baseCurrency / locale | AT / EUR / de-AT |
| meta.legalForms | gmbh, ag, og, kg, e-u |
| identifiers.vatIdFormat | /^ATU\d{8}$/i |
| identifiers.peppolSchemeId | 9914 |
| tax.standardRateBp / codes | 2000 / 20, 13, 10, V, R, RE, M, P |
| tax.accounts.ledger | [2500 Vorsteuer, 3500 Umsatzsteuer] |
| tax.accounts.fileDefault / differenceDefault | 3500 / 3500 |
| reporting.defaultChart | EKR subset, 29 accounts, zero-padded 3-digit codes |
| reporting.debtorsAccount / bankAccountDefault | 2000 / 2800 |
| compliance.filingTypes | UVA (YYYY-Qn, at-uva-quarterly), USt-Erklärung (YYYY, at-annual-vat) |
| documents.eInvoicing | peppol-bis-3.0 |
| closing | result 9350 → equity 9380 |
