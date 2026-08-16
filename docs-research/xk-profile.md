# Kosovo — bukio jurisdiction profile (XK)

Phase G (31st market). Research verified 16 August 2026. Confidence marked per item.
Note: XK is the user-assigned ISO 3166-1 alpha-2 code (not ISO-official; used by the EU
and in EN 16931 / Peppol contexts). Kosovo is NOT an EU member and NOT a Peppol participant.

## 1. VAT 2026

| Item | Value | Source / Confidence |
|---|---|---|
| Standard rate | 18% | taxatlas.io 2026, tvsh.al, ruleandlaw 2026 — HIGH (the 20% proposal from the 2025 budget debate did NOT take effect; all 2026 sources agree on 18%) |
| Reduced rate | 8% (bread, milk, essential goods) | same — HIGH |
| Zero-rated | 0% on exports | ruleandlaw 2026 — HIGH |
| Registration threshold | €30,000 annual turnover (mandatory); voluntary below | taxatlas/ruleandlaw 2026 — HIGH |
| Return frequency | Monthly | ATK guide; ruleandlaw 2026 — HIGH |
| Filing + payment deadline | 1–20 of the following month (TV form: "declaration deadline from 1 to 20 of each month") | ATK (atk-ks.org interactive guide) — HIGH → dayOfNextMonth(20) |
| Invoicing rule | Domestic invoices: Law on VAT (No. 05/L-037, amended) content requirements; no art. 226 baseline (non-EU) — 'eu-invoice-vereisten' used as the generic content baseline (same treatment as GB/US) | — MEDIUM |
| E-invoicing | NO national mandate; NOT a Peppol participant (no EAS scheme ID). Cross-border EN 16931 UBL emission works via 'peppol-bis-3.0' (RO-style comment: domestic e-invoicing N/A) | lasernet/e-invoice.be 2026 matrices omit Kosovo — HIGH |

## 2. Identifiers

| Item | Format | Confidence |
|---|---|---|
| VAT number | 'K' + 8–10 digits (e.g. K12345678) — TAK VAT certificate number starts with K | MEDIUM (no official regex published; commonly cited) → /^K\d{8,10}$/ |
| Tax ID (TIN / fiscal number) | 10 digits (TAK fiscal number on the registration certificate) | MEDIUM-HIGH → /^\d{10}$/ |
| Business registration number (NBR) | 8-digit number from ARBK/KBRA certificate (e.g. 81234567) | MEDIUM → /^\d{8}$/ |
| Peppol scheme | NONE (not a participant) | HIGH |

## 3. Company law

- Law No. 06/L-016 on Business Organizations (2018, as amended).
- Legal forms: **Sh.p.k.** (Shoqëri me përgjegjësi të kufizuar — LLC; most common), **Sh.a.** (Shoqëri aksionare — JSC), **O.P.** (ortëri e përgjithshme — general partnership), **K.P.** (komandite — limited partnership), **B.I.** (biznes individual — sole trader), degë (branch).
- Registry: ARBK (Agjencia për Regjistrimin e Bizneseve / KBRA), digital first.

## 4. Fiscal year

Calendar year (31 December) — HIGH.

## 5. Deadlines (ATK/Fryti 2026 calendars)

| Obligation | Deadline | Confidence |
|---|---|---|
| VAT return + payment | Monthly, by the 20th of the following month | HIGH |
| Corporate income tax return + final payment | 31 March of the following year (10% flat CIT; interim/prepayments 15 Apr/15 Jul/15 Oct/15 Jan) | HIGH (PwC 2026 quick chart: 31 Mar / 31 Mar; Fryti 2026) |
| Annual financial statements (AFS) | 31 March of the following year (per 2026 compliance guides; the ARBK statutory window is up to 12 months after FYE for non-listed entities — modelled as 03-31, the practical deadline) | MEDIUM-HIGH |

## 6. Chart of accounts

No statutory chart of accounts in Kosovo (SKRFI — Kosovo Accounting Standards framework,
based on IFRS for SMEs; no mandated code plan). Convention chart in Albanian (4-digit):

- 1010 Arka (cash) · 1020 Banka (bank, default)
- 2010 Klientët (debtors) · 2020 Furnitorët (suppliers)
- 2210 TVSH e hyrshme (input VAT) · 2220 TVSH e dalëshme (output VAT) · 2230 Zgjidhja e TVSH-së (settlement)
- 3010 Kapitali · 3020 Rezultatet e pashpërndara · 3030 Rezultati i vitit (closing 3030→3020)
- 4010 Të hyrat nga shitjet (sales — first income) · 4020 Të hyra të tjera
- 5010 Shpenzimet e mallrave · 5020 Shpenzimet e personelit · 5030 Shpenzimet operative ·
  5040 Amortizimi · 5050 Shpenzimet financiare · 5060 Tatimi mbi të ardhurat · 5070 Shpenzime të tjera

## 7. Currency

EUR (unilateral euro adoption since 2002; no Kosovo central bank, euro as legal tender) — HIGH.
baseCurrency 'EUR' with a comment.

## 8. i18n

Albanian (sq) — official language; full 89-key table added with this profile (docs in
Albanian by default, matching the v0.16.2 per-market pattern). Serbian is the second
official language — not offered as a table (no demand signal; sq covers the market).

## 9. B-milestones

- Domestic e-invoicing: N/A (no mandate, not Peppol) — cross-border UBL only.
- VAT return engine (TV form XML): if ever needed — but no e-filing mandate today; the
  monthly return remains a manual ATK portal filing. No engine planned.
