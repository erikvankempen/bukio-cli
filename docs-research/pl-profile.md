# Poland — bukio jurisdiction profile (PL)

Phase F profile. Research verified 16 August 2026. Confidence marked per item.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | **23 %** | EC europa.eu VAT rates (high) |
| Reduced rate | **8 %** — foodstuffs, accommodation, construction, transport | EC (high) |
| Reduced rate | **5 %** — basic foodstuffs, books | EC (high) |
| Zero rate | **0 %** — exports, intra-EU B2B | standard EU scheme (high) |
| Small business | VAT registration threshold PLN 200,000 (≈ €47K) | vatcalc PL (high) |

## 2. Identifiers

| Item | Value | Source / Confidence |
|---|---|---|
| VAT number | `PL` + **10 digits** — `/^PL\d{10}$/` (NIP with PL prefix) | Avalara EU VAT formats (high) |
| Company number | **NIP** (tax id) 10 digits; **REGON** 9 digits; **KRS** registry number (10 digits); label 'nip' | Polish registry practice (high) |
| Peppol EAS | **9945** — `PL:VAT` "Poland VAT number" | Official OpenPEPPOL EAS codelist, docs.peppol.eu (EU PINT, release 8 Dec 2025) (high) |

## 3. Legal forms

- **sp. z o.o.** — spółka z ograniczoną odpowiedzialnością (limited liability, dominant)
- **S.A.** — spółka akcyjna (joint-stock)
- **sp. j.** / **sp. k.** — general / limited partnership
- **JDG** — jednoosobowa działalność gospodarcza (sole trader)

Confidence: high (standard KSH forms).

## 4. Chart of accounts

Poland has a **statutory chart framework** — the plan kont per the Finance Minister's regulation (Rozporządzenie MF), with the classic 0-8 class structure (0 fixed assets, 1 cash, 2 receivables/payables, 3 inventory, 4 capital, 5 costs, 6 revenue, 7/8 other). Skeleton (4-digit, Polish names):

| Code | Account | Code | Account |
|---|---|---|---|
| 1310 | Rachunek bankowy (Bank) | 4000 | Kapitał podstawowy (Share capital) |
| 1010 | Kasa (Cash) | 4010 | Kapitał zapasowy (Reserve capital) |
| 2010 | Rozrachunki z odbiorcami (Trade receivables) | 4020 | Wynik finansowy lat ubiegłych (Retained earnings) |
| 2020 | Rozrachunki z dostawcami (Trade payables) | 4030 | Wynik finansowy netto (Result for the year) |
| 2310 | Rozrachunki z pracownikami (Employee payables) | 7000 | Przychody ze sprzedaży (Sales revenue) |
| 2210 | VAT naliczony (input VAT) | 7010 | Pozostałe przychody (Other income) |
| 2220 | VAT należny (output VAT) | 5000 | Zużycie materiałów i energii (Materials) |
| 2230 | Rozliczenie VAT (VAT settlement) | 5010 | Wynagrodzenia (Personnel costs) |
| 0100 | Środki trwałe (Fixed assets) | 5020 | Amortyzacja (Depreciation) |
| 0710 | Umorzenie środków trwałych (Accumulated depreciation) | 5030 | Usługi obce (Services) |
| 3300 | Materiały i towary (Inventory) | 5040 | Pozostałe koszty (Other expenses) |
| | | 5050 | Koszty finansowe (Finance costs) |
| | | 5060 | Podatek dochodowy (Corporate tax) |

Ledger pair 2210 (input, debit) / 2220 (output, credit); settlement 2230. Closing: 4030 → 4020. Confidence: medium (MF-framework skeleton, simplified).

## 5. Compliance calendar

| Obligation | Shape & deadline | Source / Confidence |
|---|---|---|
| VAT return (JPK_V7M) | **Monthly** by the **25th** of the following month (quarterly JPK_V7K for small taxpayers — 25th of the month after the quarter) | Avalara PL; Marosa (high) |
| Annual accounts | Financial statements approved within 6 months of FYE and filed with **KRS** within 15 days of approval (~15 July for calendar year) | Polish practice (medium/high) |
| Corporate income tax (CIT-8) | Annual return due **31 March** of the following year (monthly prepayments) | PwC PL (high) |
| Fiscal year end | **31 December** | practice (high) |

## 6. E-invoicing

- **Peppol participant: yes** — scheme **9945** (`PL:VAT`). Confidence: high (EAS codelist).
- **Domestic mandate: KSeF (Krajowy System e-Faktur) live Feb/Apr 2026** — mandatory structured e-invoicing for Polish domestic transactions; a KSeF builder is a B-milestone (like FatturaPA); Peppol BIS registered for cross-border.

## 7. Payment

- **SEPA** member; **PLN** (złoty) — own currency, ECB reference rates (baseCurrency 'PLN'). Confidence: high (ECB).

## 8. Gotchas

- PLN currency — amounts in złoty.
- **KSeF is live in 2026** — the biggest domestic e-invoicing mandate of the six Phase F markets (B-milestone).
- 23 % standard; 5 % band for basic foodstuffs.
- JPK_V7 monthly by the 25th — the standard compliance shape.