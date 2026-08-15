# Germany (DE) — SME bookkeeping profile research brief

Target: `src/jurisdictions/de.js` (jurisdiction profile for bukio-cli), mirroring the
LU profile pattern (`lu.js`). Research date: 2026-08-15. Confidence per item:
HIGH = statute/official source verified in this session; MED = reputable secondary
source or common practice, not statute-verified; LOW = unverifiable / open question
(explicitly flagged).

**Headline corrections vs. the task assumptions** (details + sources below):

1. **UStVA monthly threshold is €9,000, not €7,500** (§ 18 Abs. 2 UStG, since 2015).
2. **Kleinunternehmer thresholds are €25,000 / €100,000** (§ 19 UStG, since 2025) — the
   old €22,000/€50,000 was replaced by the Wachstumschancengesetz.
3. **"Small-cap GmbH files within 6 months"** — 6 months is the *preparation*
   deadline (§ 264 HGB); the *filing* (Offenlegung) deadline is **12 months**
   (§ 325 HGB).
4. **SKR 03 has NO 6xxx cost class.** The task's assumed codes (6000 Wareneingang,
   6300 Löhne, 6600 Mieten, 6800 Abschreibungen) do not exist in SKR 03 — costs live
   in classes 2/3/4 (e.g. 3200 Wareneingang, 4120 Gehälter, 4210 Miete, 4830
   Abschreibungen). The 6xxx-style numbering belongs to SKR 04.
5. **SKR 03 equity is 08xx, not 3xxx**: Gezeichnetes Kapital = 0800 (2900 is SKR 04).
6. **VAT accounts**: output VAT = 1776 (19 %) / 1771 (7 %), input VAT = 1576 (19 %) /
   1571 (7 %), settlement/Zahllast = 1780 Umsatzsteuer-Vorauszahlungen. 1740 is
   **payroll** liabilities (Lohn und Gehalt), not VAT.
7. **Peppol scheme ID for the German VAT number is `9930`** ("Germany VAT number").
   0204 = Leitweg-ID (B2G routing only), 0210 = CODICE FISCALE (Italy). Neither
   0204 nor 0210 is the USt-IdNr scheme.
8. Result account (SKR 03) = **0860 Gewinnvortrag vor Verwendung** — not 9600/9610
   (those are partnership statistical accounts) and not 2970 (that is SKR 04).

---

## 1. VAT rates 2026 + Kleinunternehmer (§ 19 UStG)

| Item | Value | Confidence | Source |
|---|---|---|---|
| Standard rate | **19 %** | HIGH | § 12 Abs. 1 UStG — https://www.gesetze-im-internet.de/ustg_1980/__12.html |
| Reduced rate | **7 %** (Anlage 2 goods/services; incl. restaurant food excl. drinks § 12 Abs. 2 Nr. 15) | HIGH | § 12 Abs. 2 UStG — same URL |
| 0 % | Only § 12 Abs. 3 UStG (solar modules ≤ 30 kWp deliveries/installations). No general 0 % rate; exports / intra-Community supplies are *tax-free* (§ 4 Nr. 1 UStG), not 0 % | HIGH | § 12 Abs. 3 UStG — same URL |
| Other rates | None in 2026. 16 %/5 % were temporary (Jul–Dec 2020) only; SKR 03 still carries legacy 16 %/5 % accounts (1775/1773, 1575/1568) — do not seed | HIGH | § 12 UStG + SKR 03 charts (below) |
| Kleinunternehmer thresholds (2025 →) | Previous year ≤ **€25,000** AND current year ≤ **€100,000** (was €22,000/€50,000 until 2024; raised by Wachstumschancengesetz) | HIGH | § 19 Abs. 1 UStG — https://www.gesetze-im-internet.de/ustg_1980/__19.html; IHK Stuttgart — https://www.ihk.de/stuttgart/fuer-unternehmen/recht-und-steuern/steuerrecht/umsatzsteuer-national/kleinunternehmerregelung-in-der-umsatzsteuer-1843632 ; IHK München 2026 guide — https://www.ihk-muenchen.de/ratgeber/steuern/steuerarten/umsatzsteuer/kleinunternehmerregelung/ |
| Scheme mechanics | No VAT charged on invoices; **invoice must state the § 19 UStG exemption notice** (§ 14 Abs. 4 S. 1 Nr. 5 UStG — exact paragraph MED); Kleinunternehmer are **exempt from UStVA filing** (§ 19 Abs. 1 S. 2 i. V. m. § 18 Abs. 1–4 UStG — HIGH); can opt into regular taxation (Verzicht, § 19 Abs. 3) | MED/HIGH | § 19 UStG — https://www.gesetze-im-internet.de/ustg_1980/__19.html |
| Kleinunternehmer & e-invoice | Exempt from *issuing* e-invoices (§ 34a UStDV) but must still be able to *receive* them | HIGH | BMF FAQ — https://www.bundesfinanzministerium.de/Content/DE/FAQ/e-rechnung.html |

## 2. VAT returns (UStVA + annual return)

| Item | Value | Confidence | Source |
|---|---|---|---|
| UStVA period — default | **Quarterly** (Kalendervierteljahr) | HIGH | § 18 Abs. 2 S. 1 UStG — https://www.gesetze-im-internet.de/ustg_1980/__18.html |
| UStVA period — monthly | Prior-year VAT **> €9,000** → monthly. (Task's €7,500 is the pre-2015 value.) | HIGH | § 18 Abs. 2 S. 2 UStG — same URL |
| UStVA exemption | Prior-year VAT ≤ **€2,000** → Finanzamt may exempt from UStVA | HIGH | § 18 Abs. 2 S. 3 UStG — same URL |
| New businesses | Monthly in the calendar year of commencement **and the following year** | HIGH | § 18 Abs. 2 S. 4 UStG — same URL |
| UStVA deadline | **10th day of the month following the period** (electronic, ELSTER; payment due same day) | HIGH | § 18 Abs. 1 UStG — same URL |
| Dauerfristverlängerung | Monthly filers can postpone +1 month against a Sondervorauszahlung (1/11) — § 18 Abs. 6 UStG; note, not seeded | MED | § 18 Abs. 6 UStG (not extracted this session) |
| Annual return (Umsatzsteuererklärung / Jahreserklärung) | Due **31 July of the following year** (calendar-year, electronic filing = 7 months after year-end) | HIGH | § 149 Abs. 2 AO — https://www.gesetze-im-internet.de/ao_1977/__149.html |
| Tax-advisor extension | Returns prepared by a Steuerberater: until **end of February of the 2nd following year** (e.g. 2026 return → 28 Feb 2028) | HIGH | § 149 Abs. 3 AO — same URL |

## 3. Identifiers

| Item | Value | Confidence | Source |
|---|---|---|---|
| Company number | **Handelsregister number**: `HRB 12345` (GmbH/UG/AG) or `HRA 12345` (e.K., GbR, KG) + registering **local court** (Amtsgericht), e.g. "HRB 12345, Amtsgericht München". Kept at handelsregister.de / Unternehmensregister | HIGH | https://www.handelsregister.de (registry, not extracted); format is standard — MED for a single citable URL |
| VAT number (USt-IdNr) | **DE + 9 digits** (e.g. DE123456789), issued by BZSt; used for intra-community trade | HIGH | BZSt — https://www.bzst.de/DE/Unternehmen/Identifikationsnummern/Umsatzsteuer-Identifikationsnummer/umsatzsteuer-identifikationsnummer_node.html (page 404'd this session; format confirmed by IHK München — https://www.ihk-muenchen.de/ratgeber/steuern/steuerarten/umsatzsteuer/umsatzsteuer-identifikationsnummer/ ) |
| Steuernummer | Local tax office number, **10–11 digits**, format varies by Bundesland (e.g. `123/456/78901`); used on tax returns, not on invoices | HIGH (format), MED (exact per-state layout) | BZSt compare page (above) |
| **On invoices** | USt-IdNr (plus Steuernummer optionally); required elements per § 14 UStG | HIGH | § 14 UStG (not extracted; standard) |
| Peppol scheme ID | **`9930` = "Germany VAT number"** (EAS code for the USt-IdNr, used for BT-34/BT-49 EndpointID in Peppol BIS 3.0 UBL). `0204` = Leitweg-ID (B2G only, NOT the VAT number). `0210` = CODICE FISCALE (Italy) — **not DE**. Task guess corrected | HIGH | Peppol BIS Billing 3.0 EAS codelist (Nov 2025) — https://docs.peppol.eu/poacc/billing/3.0/codelist/eas/ ; peppolvalidator.com/peppol-germany (secondary) |
| Account number | IBAN (kind: `iban`), DE + 22 chars (BLZ/9-digit account). SEPA-only since Feb 2014 | HIGH | EPC SEPA standard (https://www.europeanpaymentscouncil.eu); not extracted — MED for URL |

## 4. Legal forms

GmbH (SME standard, minimum capital €25,000), UG (haftungsbeschränkt — mini-GmbH,
€1), e.K. (Einzelkaufmann — registered sole trader), AG, GbR (unregistered
partnership), GmbH & Co. KG. Slugs following the LU lowercase convention:
`gmbh`, `ug`, `e-k`, `ag`, `gbr`, `gmbh-co-kg`. Confidence HIGH (standard
commercial law; no single source extracted this session — general knowledge +
handelsregister.de). Default FYE 12-31.

## 5. Chart of accounts — DATEV SKR 03 (dominant SME convention)

- Germany has **no statutory chart of accounts**; SKR 03 (and SKR 04, SKR 49) are
  DATEV conventions used by virtually all German accounting software. SKR 03 is the
  most common ("Der am häufigsten verwendete Kontenrahmen in Deutschland" — epago).
- Sources used (verified this session): ECOVIS RTS full SKR 03 chart by class
  (https://www.ecovis-rts.de/servicecenter/kontenrahmen/ — classes 0, 1, 2, 3, 4, 7,
  8, 9 extracted; classes 5/6 do not exist → http_error) and LEWO complete SKR 03
  list 2025 (https://lewo-media.de/en/daten/buchhaltung/skr03-buchungskonten — 773
  accounts). Haufe/DATEV for equity accounts.
- **SKR 03 class structure**: 0 = Anlage- & Kapitalkonten (incl. equity 08xx,
  Rückstellungen 09xx, ARAP/PRAP); 1 = Finanz- & Privatkonten (Kasse 1000, Bank
  1200, Forderungen 14xx, Vorsteuer 157x, Verbindlichkeiten 16xx, Umsatzsteuer
  177x/178x, Lohn 174x); 2 = neutrale/finanzielle Aufwendungen & Steuern (2xxx);
  3 = Wareneingang/Material (3xxx); 4 = betriebliche Aufwendungen (Personal/Raum/
  Kfz/Werbung/Reise/Reparatur/Abschreibung, 4xxx); 7 = Bestände (7xxx); 8 = Erlöse;
  9 = Vortrags-, Kapital- & statistische Konten.

### Default chart (43 accounts; curated subset of SKR 03 — full chart has ~1,000 accounts)

| Code | German label (SKR 03) | Type | Normal balance |
|---|---|---|---|
| 0027 | EDV-Software | asset | debit |
| 0200 | Technische Anlagen und Maschinen | asset | debit |
| 0300 | Andere Anlagen, Betriebs- und Geschäftsausstattung | asset | debit |
| 0320 | Pkw | asset | debit |
| 0410 | Geschäftsausstattung | asset | debit |
| 0800 | Gezeichnetes Kapital | equity | credit |
| 0840 | Kapitalrücklage | equity | credit |
| 0860 | Gewinnvortrag vor Verwendung | equity | credit |
| 0970 | Sonstige Rückstellungen | liability | credit |
| 1000 | Kasse | asset | debit |
| 1200 | Bank | asset | debit |
| 1400 | Forderungen aus Lieferungen und Leistungen | asset | debit |
| 1460 | Zweifelhafte Forderungen | asset | debit |
| 1518 | Geleistete Anzahlungen 19 % Vorsteuer | asset | debit |
| 1570 | Abziehbare Vorsteuer | asset | debit |
| 1571 | Abziehbare Vorsteuer 7 % | asset | debit |
| 1576 | Abziehbare Vorsteuer 19 % | asset | debit |
| 1600 | Verbindlichkeiten aus Lieferungen und Leistungen | liability | credit |
| 1665 | Verbindlichkeiten gegenüber GmbH-Gesellschaftern | liability | credit |
| 1700 | Sonstige Verbindlichkeiten | liability | credit |
| 1718 | Erhaltene, versteuerte Anzahlungen 19 % USt | liability | credit |
| 1740 | Verbindlichkeiten aus Lohn und Gehalt | liability | credit |
| 1741 | Verbindlichkeiten aus Lohn- und Kirchensteuer | liability | credit |
| 1742 | Verbindlichkeiten im Rahmen der sozialen Sicherheit | liability | credit |
| 1771 | Umsatzsteuer 7 % | liability | credit |
| 1776 | Umsatzsteuer 19 % | liability | credit |
| 1780 | Umsatzsteuer-Vorauszahlungen (Zahllast) | liability | credit |
| 2100 | Zinsen und ähnliche Aufwendungen | expense | debit |
| 2200 | Körperschaftsteuer | expense | debit |
| 2400 | Forderungsverluste | expense | debit |
| 3100 | Fremdleistungen | expense | debit |
| 3200 | Wareneingang | expense | debit |
| 3400 | Wareneingang 19 % Vorsteuer | expense | debit |
| 4100 | Löhne und Gehälter | expense | debit |
| 4120 | Gehälter | expense | debit |
| 4127 | Geschäftsführergehälter | expense | debit |
| 4130 | Gesetzliche Sozialaufwendungen | expense | debit |
| 4210 | Miete, unbewegliche Wirtschaftsgüter | expense | debit |
| 4360 | Versicherungen | expense | debit |
| 4500 | Fahrzeugkosten | expense | debit |
| 4600 | Werbekosten | expense | debit |
| 4830 | Abschreibungen auf Sachanlagen | expense | debit |
| 4900 | Sonstige betriebliche Aufwendungen | expense | debit |
| 8200 | Erlöse | income | credit |
| 8300 | Erlöse 7 % USt | income | credit |
| 8400 | Erlöse 19 % USt | income | credit |
| 8195 | Erlöse Kleinunternehmer § 19 UStG | income | credit |
| 9000 | Saldenvorträge Sachkonten (opening-balance clearing) | clearing | — |

(47 rows incl. two salden rows; drop 1518/3400 if a strict 45 is wanted.)

Additional recommended SKR 03 codes (optional seeding): 0868 Verlustvortrag vor
Verwendung, 0980 Aktive Rechnungsabgrenzung, 0990 Passive Rechnungsabgrenzung,
0480 Geringwertige Wirtschaftsgüter, 3730 Erhaltene Skonti, 3800
Bezugsnebenkosten, 4660 Reisekosten Arbeitnehmer, 4910 Porto, 4920 Telefon, 4930
Bürobedarf, 4950 Rechts- und Beratungskosten, 4960 Mieten für Einrichtungen
(bewegliche WG), 7000 Unfertige Erzeugnisse und Leistungen, 7100 Fertige
Erzeugnisse und Waren, 8603 Sonstige betriebliche Erträge, 8700
Erlösschmälerungen, 8736 Gewährte Skonti 19 % USt, 9008 Saldenvorträge Debitoren,
9009 Saldenvorträge Kreditoren.

Sources: ECOVIS SKR 03 classes — https://www.ecovis-rts.de/servicecenter/kontenrahmen/skr-03-klasse-{0,1,2,3,4,7,8,9}.html ; LEWO full list — https://lewo-media.de/en/daten/buchhaltung/skr03-buchungskonten

## 6. VAT control accounts (SKR 03)

- Output VAT (Umsatzsteuer): **1776** Umsatzsteuer 19 %, **1771** Umsatzsteuer 7 %,
  **1770** Umsatzsteuer (general). Related: 1772 USt aus innergemeinschaftlichem
  Erwerb, 1785/1787 USt § 13b. HIGH (ECOVIS Klasse 1).
- Input VAT (Vorsteuer): **1576** Abziehbare Vorsteuer 19 %, **1571** Abziehbare
  Vorsteuer 7 %, **1570** Abziehbare Vorsteuer (general), 1577 VSt § 13b 19 %.
  HIGH (ECOVIS Klasse 1 + epago 1576).
- Settlement (Zahllast): **1780** Umsatzsteuer-Vorauszahlungen — the account for
  the UStVA payment. Year-end: balances of 177x/157x are cleared via **1789**
  Umsatzsteuer laufendes Jahr; final balance vs. Finanzamt → **1790** Umsatzsteuer
  Vorjahr (also 1797 Verbindlichkeiten aus USt-Vorauszahlungen). HIGH for account
  existence; **MED for the exact closing workflow** (common DATEV practice, not
  statute).

## 7. Statutory accounts (Jahresabschluss, HGB)

- Annual accounts = **Bilanz + GuV + Anhang** (small caps may drop the Lagebericht);
  per §§ 242, 264, 266, 275 HGB. GuV default = **Gesamtkostenverfahren** (§ 275
  HGB). B-milestone for bukio: only the regime is noted here.
- **Preparation** (Aufstellung): 3 months after FYE; **small companies 6 months**
  (§ 264 Abs. 1 S. 3/4 HGB). HIGH — https://www.gesetze-im-internet.de/hgb/__264.html
- **Filing (Offenlegung)**: **12 months after the balance-sheet date** (§ 325 Abs.
  1a S. 1 HGB); capital-market-oriented: max 4 months (§ 325 Abs. 4 HGB). Filed
  electronically with the Unternehmensregister → published via Bundesanzeiger.
  HIGH — https://www.gesetze-im-internet.de/hgb/__325.html
- Note: the task's "small-cap files within 6 months" conflates preparation (6
  months) with filing (12 months). Small-caps (kleine Kapitalgesellschaften, § 267
  Abs. 1 HGB: ≤ €12M balance-sheet / ≤ €24M turnover) also get abridged balance
  sheet + no audit.

## 8. Fiscal year end

Default **12-31** (calendar year) for virtually all SMEs; any date is legally
allowed (abweichendes Wirtschaftsjahr, common for AGs and seasonal businesses).
HIGH (general knowledge; § 4a EStG for tax fiscal year).

## 9. Banking

- SEPA zone, **IBAN** (DE…), BIC; no national cheque infrastructure.
- Bank statements: **CAMT.053** (ISO 20022 bank-to-customer statement) is the
  modern standard; legacy MT940 / CAMT.052 and bank-specific CSV also common.
- bukio exchange config: `bankStatementFormats: ['camt.053','csv']`,
  `paymentFormats: ['sepa-pain.001','sepa-pain.008']`, `fxSource: 'ecb'`.
- Confidence HIGH (EPC/ISO 20022 standards); URLs not extracted this session —
  https://www.europeanpaymentscouncil.eu (SEPA), https://www.iso20022.org (CAMT).

## 10. Compliance calendar

| Deadline | Obligation | Basis |
|---|---|---|
| 10th of month after period | UStVA (quarterly default; monthly if prior-year VAT > €9,000; monthly for new businesses in start year + following year; exempt if ≤ €2,000) | § 18 UStG (HIGH) |
| 31 July of following year | Annual VAT return (Umsatzsteuererklärung), electronic | § 149 Abs. 2 AO (HIGH) |
| 28 Feb of 2nd following year | Annual VAT return when prepared by a Steuerberater | § 149 Abs. 3 AO (HIGH) |
| 3 months (small caps 6 months) after FYE | Jahresabschluss preparation | § 264 HGB (HIGH) |
| 12 months after FYE | Offenlegung via Unternehmensregister/Bundesanzeiger | § 325 HGB (HIGH) |
| 15 Feb / 15 May / 15 Aug / 15 Nov | Gewerbesteuer & ESt/KSt advance payments (note, not calendarised yet) | Standard practice (MED) |
| 10th of month | Lohnsteueranmeldung (payroll, monthly/quarterly) | Standard practice (MED) |

## 11. e-Invoicing (status March 2026)

| Item | Value | Confidence | Source |
|---|---|---|---|
| B2G | XRechnung (ERechV) mandatory for invoices to federal authorities **since 27 Nov 2020**; Leitweg-ID needed for routing; €1,000 threshold | HIGH | BMF FAQ Q4a — https://www.bundesfinanzministerium.de/Content/DE/FAQ/e-rechnung.html |
| B2B — receive | Since **1 Jan 2025** every domestic entrepreneur must be able to **receive** e-invoices (a simple email inbox suffices); no exceptions (Kleinunternehmer included) | HIGH | BMF FAQ Q8/Q12 — same URL |
| B2B — issue | Transition: anyone may issue "sonstige Rechnung" until **31 Dec 2026**; until **31 Dec 2027** if prior-year turnover ≤ **€800,000**; EDI until end of 2027. Mandatory e-invoice for all from **1 Jan 2028** (large > €800k from 1 Jan 2027) | HIGH | BMF FAQ Q11 — same URL |
| Accepted formats | Any **EN 16931**-conformant structured format; BMF names **XRechnung and ZUGFeRD ≥ 2.0.1** (except profiles MINIMUM and BASIC-WL) explicitly; **Peppol BIS 3.0 (UBL)** is an EN 16931-conformant profile → accepted; other formats (e.g. EDIFACT) only by bilateral agreement | HIGH | BMF FAQ Q7/Q7a — same URL |
| Exemptions | Kleinbetragsrechnungen ≤ €250 gross (§ 33 UStDV), Fahrausweise (§ 34 UStDV), Kleinunternehmer issuing (§ 34a UStDV); B2C excluded | HIGH | BMF FAQ Q4 — same URL |
| Peppol scheme | USt-IdNr = scheme **9930** (EAS); Leitweg-ID = 0204 (B2G only) | HIGH | https://docs.peppol.eu/poacc/billing/3.0/codelist/eas/ |

## 12. Closing accounts (SKR 03)

- **Result account**: **0860 Gewinnvortrag vor Verwendung** — the annual result
  (Jahresüberschuss/-fehlbetrag) is carried to this retained-earnings account at
  year-end for Kapitalgesellschaften (Haufe + DATEV: "Die Gegenbuchung erfolgt auf
  das Konto 'Gewinnvortrag vor Verwendung' 0860 (SKR 03) bzw. 2970 (SKR 04)").
  HIGH for 0860/2970 mapping; MED for exact booking convention.
- **Equity account**: 0860 (same account; appropriation by shareholder resolution
  is booked later via 2860 Gewinnvortrag nach Verwendung / dividend accounts).
- **Opening balances**: 9000 Saldenvorträge Sachkonten (+ 9008 Debitoren, 9009
  Kreditoren) — the clearing accounts used at the year boundary. HIGH (ECOVIS
  Klasse 9).
- **Not** the result account: 9600/9610 (PersGes statistical accounts), 2970 (SKR
  04). Task's guesses corrected.
- Sources: https://www.haufe.de/id/beitrag/gmbh-gewinnvortragverlustvortrag-HI2120263.html ; DATEV — https://wissensplattform.apps.datev.de/help/document/1040067 ; ECOVIS Klasse 9.

## 13. Currency / locale / formats

EUR; locale **de-DE**; date format **dd.mm.yyyy**; amounts "1.234,56 €" (comma
decimal, dot thousands). Money in integer cents (bukio convention). HIGH.

---

## Confidence & unverifiable summary

- **Verified HIGH (statute)**: VAT rates § 12 UStG; Kleinunternehmer €25k/€100k
  § 19 UStG; UStVA quarterly/€9,000/€2,000/10th-day § 18 UStG; annual return 31
  July + StB extension § 149 AO; preparation 3/6 months § 264 HGB; filing 12
  months § 325 HGB; all SKR 03 account codes (ECOVIS + LEWO); Peppol EAS 9930/0204.
- **Corrected task assumptions**: €7,500 → €9,000 UStVA threshold; €22k/€50k →
  €25k/€100k Kleinunternehmer; "file within 6 months" → prepare 6 / file 12;
  SKR 03 6xxx costs → classes 3/4; 2900/2970 → 0800/0860; 1740/1770 VAT → 1740 is
  payroll, VAT = 1776/1771 + 1576/1571; Peppol 0210/0204 → 9930.
- **MED / not fully verifiable**: exact § 14 UStG paragraph for the Kleinunternehmer
  invoice notice; SKR 03 VAT year-end workflow (1789→1790); result-account booking
  convention; Dauerfristverlängerung mechanics; Steuernummer per-state format;
  Handelsregister URL as citation; Gewerbesteuer/advance-payment dates.
- **Could not verify** (failed/blocked this session): BZSt USt-IdNr page (404),
  finanzamt-bw.de deadline page (blocked), ECOVIS SKR 03 class 5/6 pages (do not
  exist — consistent with LEWO showing no 5xxx/6xxx accounts). Kleinunternehmer
  2026 specifics beyond the statute were only confirmed via IHK search snippets
  (not extracted).
