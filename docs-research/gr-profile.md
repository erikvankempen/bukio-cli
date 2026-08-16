# Greece — bukio jurisdiction profile (GR)

Phase F profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **24 %** | EC europa.eu VAT rates (high) |
| Reduced rate | **13 %** — foodstuffs, accommodation, utilities, transport | EC (high) |
| Reduced rate | **6 %** — books, medicines, theatre, newspapers | EC (high) |
| Zero rate | **0 %** — exports, intra-EU B2B | standard EU scheme (high) |
| Small business | VAT registration threshold **€10,000** (small-scale exemption, art. 39b Greek VAT Code) | vatcalc GR (high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `EL` + **9 digits** — `/^EL\d{9}$/` (Greece uses **EL** as the VIES prefix, not GR) | Docuflair; Avalara; LookupTax (high) |
| Company number | **AFM** (ΑΦΜ, tax number) — 9 digits; **GEMI** registration number (Γ.Ε.ΜΗ.) for the company registry; label 'afm' | businessdataguide; GEMI (high) |
| Peppol EAS | **9933** — `EL:VAT` "Greece VAT number" | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **IKE** (ΙΚΕ) — private capital company (dominant for new SMEs)
- **EPE** (ΕΠΕ) — limited liability
- **AE** (ΑΕ) — société anonyme (joint-stock)
- **OE / ΕΕ** — general / limited partnership
- **ατομική επιχείρηση** — sole trader

Confidence: high (standard Greek company-law forms).

## 4. Chart of accounts

Greece has a **statutory chart** — the Ελληνικό Γενικό Λογιστικό Σχέδιο (ΕΓΛΣ, Greek GAAP chart, PD 1123/1980, 3-digit scheme with subaccounts). The bukio skeleton uses the ΕΓΛΣ primary groups (3 cash/receivables, 4 equity/liabilities, 5 expenses, 6 income — the old scheme; 2023+ GAAP editions renumber) as a 4-digit convention:

| Code | Account | Code | Account |
|---|---|---|---|
| 3802 | Τράπεζες (Bank) | 4000 | Κεφάλαιο (Share capital) |
| 3800 | Ταμείο (Cash) | 4200 | Αποτελέσματα εις νέον (Retained earnings) |
| 3000 | Πελάτες (Trade receivables) | 4300 | Αποτέλεσμα χρήσης (Result for the year) |
| 5000 | Προμηθευτές (Trade payables) | 8000 | Πωλήσεις (Sales revenue) |
| 5300 | Αμοιβές προσωπικού πληρωτέες (Employee payables) | 8100 | Λοιπά έσοδα (Other income) |
| 5400 | ΦΠΑ εισροών (input VAT) | 6000 | Αγορές εμπορευμάτων (Purchases) |
| 5450 | ΦΠΑ εκροών (output VAT) | 6100 | Αμοιβές προσωπικού (Personnel costs) |
| 5403 | ΦΠΑ — εκκαθάριση (VAT settlement) | 6200 | Παροχές τρίτων (Services) |
| 1000 | Ενσώματα πάγια (Fixed assets) | 6600 | Αποσβέσεις (Depreciation) |
| 1100 | Αποσβεσμένα πάγια (Accumulated depreciation) | 6500 | Λοιπά έξοδα (Other expenses) |
| 2000 | Αποθέματα (Inventory) | 6700 | Χρηματοοικονομικά έξοδα (Finance costs) |
| | | 6900 | Φόρος εισοδήματος (Corporate tax) |

Ledger pair 5400 (input, debit) / 5450 (output, credit); settlement 5403. Closing: 4300 → 4200. Confidence: medium (ΕΓΛΣ-structured skeleton; modern editions renumber).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return | **Monthly** by the **26th** of the following month; quarterly filers by the **30th** of the month after the quarter (myDATA-based filing) | Firmbee 2026 (medium/high) |
| Annual accounts | Financial statements published/filed (GEMI) within **10 months** of FYE (31 October for calendar year) | Greek practice (medium/high) |
| Corporate income tax | Annual CIT return (ΝΠΟ) filed by **30 June** of the following year (instalment payments through the year) | PwC GR (medium/high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: yes** — scheme **9933** (`EL:VAT`). Confidence: high (EAS codelist).
- **Domestic mandate: myDATA electronic books/reporting is mandatory** (transactions reported digitally to AADE) — a myDATA reporting engine is a B-milestone; Peppol BIS registered for cross-border.

## 7. Payment

- **SEPA** member; **EUR** (eurozone since 2001). IBAN-based payments. Confidence: high (ECB).

## 8. Gotchas

- **EL prefix** (not GR) for the VAT/VIES number.
- 24 % standard — one of the EU's highest; 6 % super-reduced for books/medicines.
- myDATA mandatory digital books (B-milestone) — transactions must be reported digitally.
- Small-business VAT threshold just €10,000 — most sole traders are exempt.