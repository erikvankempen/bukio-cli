/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Central i18n mechanism (S1, owner decision 15 Aug 2026):
//   - t(key, params, locale) resolves a key against the locale table with
//     fallbacks: exact locale -> language part -> 'en' -> the key itself.
//     Regional overrides (nl-be, fr-lu) hold only the keys that differ and
//     fall back to their base language (nl, fr).
//   - resolveLocale(ctx, db) picks the active UI locale: --locale flag (or
//     BUKIO_LOCALE env) -> 'en'. UI text is English by default — the
//     company's stored locale drives DOCUMENTS (invoice.language), not the
//     CLI surface; localization is an explicit opt-in.
//   - Supported tables: en (default; also serves en-GB/en-US), nl (NL),
//     nl-be (BE Dutch), de (DE), fr (FR; base for fr-lu), fr-lu (LU),
//     da (DK), fi (FI), nb (NO), sv (SE).
//   - Statutory artifacts (XAF/FAIA, jaarrekening models, OB readout),
//     JSON output keys, error codes and MCP descriptions never localize
//     (per the Aug 14 decision: documents localize, UI/JSON stay English).
// Line descriptions and account names are data, never auto-translated.

export const UNIT_CODES = ['h', 'day', 'month', 'unit', 'session', 'km', 'kg', 'project'];

export const TABLES = {
  en: {
    // --- invoice PDF / document labels (structural labels only) ---
    'pdf.invoice': 'INVOICE', 'pdf.credit': 'CREDIT NOTE',
    'pdf.billedTo': 'Billed to', 'pdf.date': 'Date', 'pdf.dueDate': 'Due date',
    'pdf.reference': 'Reference', 'pdf.kvk': 'CoC', 'pdf.btw': 'VAT',
    'pdf.description': 'Description', 'pdf.qty': 'Qty', 'pdf.unit': 'Unit', 'pdf.price': 'Price',
    'pdf.vat': 'VAT', 'pdf.discount': 'Discount', 'pdf.amount': 'Amount',
    'pdf.subtotal': 'Subtotal excl. VAT', 'pdf.vatOn': 'VAT on', 'pdf.vatTotal': 'Total VAT',
    'pdf.total': 'Total', 'pdf.inclVat': 'incl. VAT',
    'pdf.footerPay': 'Please transfer the amount within {term} to IBAN {iban} for the account of {name}.',
    'pdf.dueDateTerm': '{date}', 'pdf.defaultTerm': 'the agreed term',
    // --- units ---
    'unit.h': 'h', 'unit.day': 'day', 'unit.month': 'month', 'unit.unit': 'unit',
    'unit.session': 'session', 'unit.km': 'km', 'unit.kg': 'kg', 'unit.project': 'project',
    // --- statuses & directions ---
    'status.draft': 'draft', 'status.posted': 'posted', 'status.open': 'open',
    'status.closed': 'closed', 'status.overdue': 'overdue', 'status.paid': 'paid',
    'dir.payable': 'payable', 'dir.receivable': 'receivable', 'dir.debit': 'debit', 'dir.credit': 'credit',
    // --- report labels (generic reports) ---
    'report.revenue': 'revenue', 'report.costs': 'costs', 'report.result': 'result',
    'report.undistributedResult': 'undistributed result', 'report.totalLiabilities': 'total liabilities',
    'report.netResult': 'net result', 'report.profitAndLoss': 'PROFIT AND LOSS', 'report.pnlSheet': 'Profit and loss',
    // --- vat file / settle descriptions ---
    'vat.file.description': 'VAT return{period} — transfer to {account} ({direction})',
    'vat.settle.description': 'VAT return payment{period} — {account} (rounding difference {amount})',
    // --- emails ---
    'email.invoiceSubject': 'Invoice {number} — {company}',
    'email.invoiceBody': 'Dear client,\n\nPlease find attached invoice {number} for a total of {gross} (incl. VAT).\n\nKind regards,\n{company}',
    'email.reminderSubject': 'Payment reminder invoice {number}',
    'email.reminderBody': 'Dear {name},\n\nInvoice {number} still has {outstanding} outstanding (due {dueDate}).\n{transfer}\n\nKind regards,\n{company}',
    'email.reminderTransferIban': 'Please transfer the amount to IBAN {iban} with reference {number}.',
    'email.reminderTransferPlain': 'Please transfer the outstanding amount with reference {number}.',
    // --- invoice list / reminders tables ---
    'invlist.number': 'number', 'invlist.type': 'type', 'invlist.date': 'date',
    'invlist.customer': 'customer', 'invlist.total': 'total', 'invlist.status': 'status',
    'invlist.days': 'days', 'invlist.reminder': 'reminder',
    'reminder.none': 'no reminders as of {date}',
    'entry.paid': 'paid {date}: {amount} ({method})',
    'entry.outstanding': 'outstanding: {amount}',
    // --- month-end / year-end renders ---
    'monthend.totals': 'totals:   debit {debit} / credit {credit} {state}',
    'yearend.plan': 'plan: close {year} — result {amount}{extra}',
    'yearend.closed': '{year} closed — result {amount} (entries #{entries}, posted)',
    'yearend.status': '{year}: {state} — result {amount}',
  },

  nl: {
    // --- invoice PDF / document labels (structural labels only) ---
    'pdf.invoice': 'FACTUUR', 'pdf.credit': 'CREDITFACTUUR',
    'pdf.billedTo': 'Factuur aan', 'pdf.date': 'Datum', 'pdf.dueDate': 'Vervaldatum',
    'pdf.reference': 'Referentie', 'pdf.kvk': 'KvK', 'pdf.btw': 'BTW',
    'pdf.description': 'Omschrijving', 'pdf.qty': 'Aantal', 'pdf.unit': 'Eenheid', 'pdf.price': 'Prijs',
    'pdf.vat': 'Btw', 'pdf.discount': 'Korting', 'pdf.amount': 'Bedrag',
    'pdf.subtotal': 'Subtotaal excl. btw', 'pdf.vatOn': 'Btw over', 'pdf.vatTotal': 'Totaal btw',
    'pdf.total': 'Totaal', 'pdf.inclVat': 'incl. btw',
    'pdf.footerPay': 'Gelieve het bedrag binnen {term} over te maken op IBAN {iban} t.n.v. {name}.',
    'pdf.dueDateTerm': '{date}', 'pdf.defaultTerm': 'de gestelde termijn',
    // --- units ---
    'unit.h': 'uur', 'unit.day': 'dag', 'unit.month': 'maand', 'unit.unit': 'stuks',
    'unit.session': 'sessie', 'unit.km': 'km', 'unit.kg': 'kg', 'unit.project': 'project',
    // --- statuses & directions ---
    'status.draft': 'concept', 'status.posted': 'geboekt', 'status.open': 'open',
    'status.closed': 'gesloten', 'status.overdue': 'vervallen', 'status.paid': 'betaald',
    'dir.payable': 'te betalen', 'dir.receivable': 'te ontvangen', 'dir.debit': 'debet', 'dir.credit': 'credit',
    // --- report labels (generic reports) ---
    'report.revenue': 'opbrengsten', 'report.costs': 'kosten', 'report.result': 'resultaat',
    'report.undistributedResult': 'Nog te verdelen resultaat', 'report.totalLiabilities': 'totaal passiva',
    'report.netResult': 'Netto resultaat', 'report.profitAndLoss': 'WINST- EN VERLIESREKENING', 'report.pnlSheet': 'Winst en verlies',
    // --- vat file / settle descriptions ---
    'vat.file.description': 'OB-aangifte{period} verlegging naar {account} ({direction})',
    'vat.settle.description': 'Betaling OB-aangifte{period} — {account} (afrondingsverschil {amount})',
    // --- emails ---
    'email.invoiceSubject': 'Factuur {number} — {company}',
    'email.invoiceBody': 'Geachte,\n\nHierbij ontvangt u factuur {number} voor een totaalbedrag van {gross} (incl. btw).\n\nMet vriendelijke groet,\n{company}',
    'email.reminderSubject': 'Betalingsherinnering factuur {number}',
    'email.reminderBody': 'Beste {name},\n\nVoor factuur {number} staat nog {outstanding} open (vervaldatum {dueDate}).\n{transfer}\n\nMet vriendelijke groet,\n{company}',
    'email.reminderTransferIban': 'Wilt u dit bedrag overmaken naar IBAN {iban} o.v.v. {number}?',
    'email.reminderTransferPlain': 'Wilt u het openstaande bedrag overmaken o.v.v. {number}?',
    // --- invoice list / reminders tables ---
    'invlist.number': 'nummer', 'invlist.type': 'type', 'invlist.date': 'datum',
    'invlist.customer': 'klant', 'invlist.total': 'totaal', 'invlist.status': 'status',
    'invlist.days': 'dagen', 'invlist.reminder': 'herinnering',
    'reminder.none': 'geen herinneringen per {date}',
    'entry.paid': 'betaald {date}: {amount} ({method})',
    'entry.outstanding': 'openstaand: {amount}',
    // --- month-end / year-end renders ---
    'monthend.totals': 'totalen:  debet {debit} / credit {credit} {state}',
    'yearend.plan': 'plan: sluit {year} — resultaat {amount}{extra}',
    'yearend.closed': '{year} gesloten — resultaat {amount} (boekingen #{entries}, geboekt)',
    'yearend.status': '{year}: {state} — resultaat {amount}',
  },

  // --- Belgian Dutch (nl-BE): regional override of nl. BTW not
  //     omzetbelasting, KBO not KvK, BTW-aangifte not OB-aangifte. ---
  'nl-be': {
    'pdf.credit': 'CREDITNOTA', 'pdf.kvk': 'KBO', 'pdf.btw': 'BTW', 'pdf.vat': 'BTW',
    'vat.file.description': 'BTW-aangifte{period} — overdracht naar {account} ({direction})',
    'vat.settle.description': 'Betaling BTW-aangifte{period} — {account} (afrondingsverschil {amount})',
    'email.reminderSubject': 'Aanmaning factuur {number}',
    'report.undistributedResult': 'te bestemmen resultaat',
    'invlist.reminder': 'aanmaning',
  },

  // --- German (de-DE) ---
  de: {
    'pdf.invoice': 'RECHNUNG', 'pdf.credit': 'GUTSCHRIFT',
    'pdf.billedTo': 'Rechnung an', 'pdf.date': 'Datum', 'pdf.dueDate': 'Fälligkeitsdatum',
    'pdf.reference': 'Referenz', 'pdf.kvk': 'Handelsregister', 'pdf.btw': 'MwSt.',
    'pdf.description': 'Beschreibung', 'pdf.qty': 'Menge', 'pdf.unit': 'Einheit', 'pdf.price': 'Preis',
    'pdf.vat': 'MwSt.', 'pdf.discount': 'Rabatt', 'pdf.amount': 'Betrag',
    'pdf.subtotal': 'Zwischensumme exkl. MwSt.', 'pdf.vatOn': 'MwSt. auf', 'pdf.vatTotal': 'MwSt. gesamt',
    'pdf.total': 'Gesamt', 'pdf.inclVat': 'inkl. MwSt.',
    'pdf.footerPay': 'Bitte überweisen Sie den Betrag innerhalb von {term} auf IBAN {iban} zugunsten von {name}.',
    'pdf.dueDateTerm': '{date}', 'pdf.defaultTerm': 'der vereinbarten Frist',
    'unit.h': 'Std.', 'unit.day': 'Tag', 'unit.month': 'Monat', 'unit.unit': 'Stk.',
    'unit.session': 'Sitzung', 'unit.km': 'km', 'unit.kg': 'kg', 'unit.project': 'Projekt',
    'status.draft': 'Entwurf', 'status.posted': 'gebucht', 'status.open': 'offen',
    'status.closed': 'geschlossen', 'status.overdue': 'überfällig', 'status.paid': 'bezahlt',
    'dir.payable': 'zu zahlen', 'dir.receivable': 'zu erhalten', 'dir.debit': 'Soll', 'dir.credit': 'Haben',
    'report.revenue': 'Erlöse', 'report.costs': 'Kosten', 'report.result': 'Ergebnis',
    'report.undistributedResult': 'Bilanzgewinn', 'report.totalLiabilities': 'Passiva gesamt',
    'report.netResult': 'Nettoergebnis', 'report.profitAndLoss': 'GEWINN- UND VERLUSTRECHNUNG', 'report.pnlSheet': 'Gewinn- und Verlustrechnung',
    'vat.file.description': 'Umsatzsteuervoranmeldung{period} — Umbuchung auf {account} ({direction})',
    'vat.settle.description': 'Zahlung der Umsatzsteuervoranmeldung{period} — {account} (Rundungsdifferenz {amount})',
    'email.invoiceSubject': 'Rechnung {number} — {company}',
    'email.invoiceBody': 'Sehr geehrte Damen und Herren,\n\nanbei erhalten Sie die Rechnung {number} über insgesamt {gross} (inkl. MwSt.).\n\nMit freundlichen Grüßen,\n{company}',
    'email.reminderSubject': 'Zahlungserinnerung Rechnung {number}',
    'email.reminderBody': 'Sehr geehrte/r {name},\n\nfür die Rechnung {number} ist noch {outstanding} offen (fällig am {dueDate}).\n{transfer}\n\nMit freundlichen Grüßen,\n{company}',
    'email.reminderTransferIban': 'Bitte überweisen Sie den Betrag auf IBAN {iban} mit dem Verwendungszweck {number}.',
    'email.reminderTransferPlain': 'Bitte überweisen Sie den offenen Betrag mit dem Verwendungszweck {number}.',
    'invlist.number': 'Nummer', 'invlist.type': 'Typ', 'invlist.date': 'Datum',
    'invlist.customer': 'Kunde', 'invlist.total': 'Gesamt', 'invlist.status': 'Status',
    'invlist.days': 'Tage', 'invlist.reminder': 'Erinnerung',
    'reminder.none': 'keine Erinnerungen zum {date}',
    'entry.paid': 'bezahlt {date}: {amount} ({method})',
    'entry.outstanding': 'offen: {amount}',
    'monthend.totals': 'Summen:  Soll {debit} / Haben {credit} {state}',
    'yearend.plan': 'Plan: Jahresabschluss {year} — Ergebnis {amount}{extra}',
    'yearend.closed': '{year} abgeschlossen — Ergebnis {amount} (Buchungen #{entries}, gebucht)',
    'yearend.status': '{year}: {state} — Ergebnis {amount}',
  },

  // --- French (fr; base for fr-lu) ---
  fr: {
    'pdf.invoice': 'FACTURE', 'pdf.credit': 'AVOIR',
    'pdf.billedTo': 'Facturé à', 'pdf.date': 'Date', 'pdf.dueDate': 'Échéance',
    'pdf.reference': 'Référence', 'pdf.kvk': 'SIREN', 'pdf.btw': 'TVA',
    'pdf.description': 'Désignation', 'pdf.qty': 'Qté', 'pdf.unit': 'Unité', 'pdf.price': 'Prix',
    'pdf.vat': 'TVA', 'pdf.discount': 'Remise', 'pdf.amount': 'Montant',
    'pdf.subtotal': 'Sous-total HT', 'pdf.vatOn': 'TVA sur', 'pdf.vatTotal': 'Total TVA',
    'pdf.total': 'Total', 'pdf.inclVat': 'TTC',
    'pdf.footerPay': 'Merci de régler le montant sous {term} sur l’IBAN {iban} au nom de {name}.',
    'pdf.dueDateTerm': '{date}', 'pdf.defaultTerm': 'le délai convenu',
    'unit.h': 'h', 'unit.day': 'jour', 'unit.month': 'mois', 'unit.unit': 'unité',
    'unit.session': 'séance', 'unit.km': 'km', 'unit.kg': 'kg', 'unit.project': 'projet',
    'status.draft': 'brouillon', 'status.posted': 'comptabilisé', 'status.open': 'ouvert',
    'status.closed': 'clôturé', 'status.overdue': 'en retard', 'status.paid': 'payé',
    'dir.payable': 'à payer', 'dir.receivable': 'à recevoir', 'dir.debit': 'débit', 'dir.credit': 'crédit',
    'report.revenue': 'produits', 'report.costs': 'charges', 'report.result': 'résultat',
    'report.undistributedResult': 'résultat à répartir', 'report.totalLiabilities': 'total du passif',
    'report.netResult': 'résultat net', 'report.profitAndLoss': 'COMPTE DE RÉSULTAT', 'report.pnlSheet': 'Compte de résultat',
    'vat.file.description': 'Déclaration de TVA{period} — transfert vers {account} ({direction})',
    'vat.settle.description': 'Paiement de la déclaration de TVA{period} — {account} (écart d’arrondi {amount})',
    'email.invoiceSubject': 'Facture {number} — {company}',
    'email.invoiceBody': 'Bonjour,\n\nVeuillez trouver ci-joint la facture {number} pour un montant de {gross} (TTC).\n\nCordialement,\n{company}',
    'email.reminderSubject': 'Rappel de paiement — facture {number}',
    'email.reminderBody': 'Bonjour {name},\n\nLa facture {number} est encore impayée à hauteur de {outstanding} (échéance le {dueDate}).\n{transfer}\n\nCordialement,\n{company}',
    'email.reminderTransferIban': 'Merci de virer le montant sur l’IBAN {iban} avec la référence {number}.',
    'email.reminderTransferPlain': 'Merci de virer le montant impayé avec la référence {number}.',
    'invlist.number': 'N°', 'invlist.type': 'Type', 'invlist.date': 'Date',
    'invlist.customer': 'Client', 'invlist.total': 'Total', 'invlist.status': 'Statut',
    'invlist.days': 'Jours', 'invlist.reminder': 'Relance',
    'reminder.none': 'aucune relance au {date}',
    'entry.paid': 'payé {date}: {amount} ({method})',
    'entry.outstanding': 'impayé: {amount}',
    'monthend.totals': 'Totaux :  débit {debit} / crédit {credit} {state}',
    'yearend.plan': 'Plan : clôture {year} — résultat {amount}{extra}',
    'yearend.closed': '{year} clôturé — résultat {amount} (écritures #{entries}, comptabilisées)',
    'yearend.status': '{year}: {state} — résultat {amount}',
  },

  // --- Luxembourg (fr-lu): regional override of fr (RCS not SIREN). ---
  'fr-lu': {
    'pdf.kvk': 'RCS',
  },

  // --- Danish (da-DK) ---
  da: {
    'pdf.invoice': 'FAKTURA', 'pdf.credit': 'KREDITNOTA',
    'pdf.billedTo': 'Faktureret til', 'pdf.date': 'Dato', 'pdf.dueDate': 'Forfaldsdato',
    'pdf.reference': 'Reference', 'pdf.kvk': 'CVR', 'pdf.btw': 'Moms',
    'pdf.description': 'Beskrivelse', 'pdf.qty': 'Antal', 'pdf.unit': 'Enhed', 'pdf.price': 'Pris',
    'pdf.vat': 'Moms', 'pdf.discount': 'Rabat', 'pdf.amount': 'Beløb',
    'pdf.subtotal': 'Subtotal ekskl. moms', 'pdf.vatOn': 'Moms på', 'pdf.vatTotal': 'Moms i alt',
    'pdf.total': 'I alt', 'pdf.inclVat': 'inkl. moms',
    'pdf.footerPay': 'Bedes venligst overført inden {term} til IBAN {iban} til fordel for {name}.',
    'pdf.dueDateTerm': '{date}', 'pdf.defaultTerm': 'den aftalte frist',
    'unit.h': 't.', 'unit.day': 'dag', 'unit.month': 'måned', 'unit.unit': 'stk.',
    'unit.session': 'session', 'unit.km': 'km', 'unit.kg': 'kg', 'unit.project': 'projekt',
    'status.draft': 'kladde', 'status.posted': 'bogført', 'status.open': 'åben',
    'status.closed': 'lukket', 'status.overdue': 'forfalden', 'status.paid': 'betalt',
    'dir.payable': 'skyldig', 'dir.receivable': 'tilgodehavende', 'dir.debit': 'debet', 'dir.credit': 'kredit',
    'report.revenue': 'omsætning', 'report.costs': 'omkostninger', 'report.result': 'resultat',
    'report.undistributedResult': 'ufordelt resultat', 'report.totalLiabilities': 'passiver i alt',
    'report.netResult': 'nettoresultat', 'report.profitAndLoss': 'RESULTATOPGØRELSE', 'report.pnlSheet': 'Resultatopgørelse',
    'vat.file.description': 'Momsangivelse{period} — overførsel til {account} ({direction})',
    'vat.settle.description': 'Betaling af momsangivelse{period} — {account} (afrundingsdifference {amount})',
    'email.invoiceSubject': 'Faktura {number} — {company}',
    'email.invoiceBody': 'Kære kunde,\n\nVedlagt er faktura {number} på i alt {gross} (inkl. moms).\n\nMed venlig hilsen,\n{company}',
    'email.reminderSubject': 'Betalingspåmindelse — faktura {number}',
    'email.reminderBody': 'Kære {name},\n\nFaktura {number} er endnu ikke betalt med {outstanding} (forfalden {dueDate}).\n{transfer}\n\nMed venlig hilsen,\n{company}',
    'email.reminderTransferIban': 'Bedes venligst overført til IBAN {iban} med angivelse af {number}.',
    'email.reminderTransferPlain': 'Bedes venligst overført det udestående beløb med angivelse af {number}.',
    'invlist.number': 'Nummer', 'invlist.type': 'Type', 'invlist.date': 'Dato',
    'invlist.customer': 'Kunde', 'invlist.total': 'I alt', 'invlist.status': 'Status',
    'invlist.days': 'Dage', 'invlist.reminder': 'Påmindelse',
    'reminder.none': 'ingen påmindelser pr. {date}',
    'entry.paid': 'betalt {date}: {amount} ({method})',
    'entry.outstanding': 'udestående: {amount}',
    'monthend.totals': 'Totaler:  debet {debit} / kredit {credit} {state}',
    'yearend.plan': 'Plan: afslut {year} — resultat {amount}{extra}',
    'yearend.closed': '{year} afsluttet — resultat {amount} (posteringer #{entries}, bogført)',
    'yearend.status': '{year}: {state} — resultat {amount}',
  },

  // --- Finnish (fi-FI) ---
  fi: {
    'pdf.invoice': 'LASKU', 'pdf.credit': 'HYVITYSLASKU',
    'pdf.billedTo': 'Laskutettu', 'pdf.date': 'Päivämäärä', 'pdf.dueDate': 'Eräpäivä',
    'pdf.reference': 'Viite', 'pdf.kvk': 'Y-tunnus', 'pdf.btw': 'Alv',
    'pdf.description': 'Kuvaus', 'pdf.qty': 'Määrä', 'pdf.unit': 'Yksikkö', 'pdf.price': 'Hinta',
    'pdf.vat': 'Alv', 'pdf.discount': 'Alennus', 'pdf.amount': 'Summa',
    'pdf.subtotal': 'Välisumma ilman alv:a', 'pdf.vatOn': 'Alv', 'pdf.vatTotal': 'Alv yhteensä',
    'pdf.total': 'Yhteensä', 'pdf.inclVat': 'sis. alv',
    'pdf.footerPay': 'Pyydämme maksamaan summan {term} kuluessa IBAN-tilille {iban} tilinhaltijan {name} hyväksi.',
    'pdf.dueDateTerm': '{date}', 'pdf.defaultTerm': 'sovitun määräajan',
    'unit.h': 'h', 'unit.day': 'päivä', 'unit.month': 'kuukausi', 'unit.unit': 'kpl',
    'unit.session': 'istunto', 'unit.km': 'km', 'unit.kg': 'kg', 'unit.project': 'projekti',
    'status.draft': 'luonnos', 'status.posted': 'kirjattu', 'status.open': 'avoin',
    'status.closed': 'suljettu', 'status.overdue': 'myöhässä', 'status.paid': 'maksettu',
    'dir.payable': 'maksettava', 'dir.receivable': 'saatava', 'dir.debit': 'debet', 'dir.credit': 'kredit',
    'report.revenue': 'tuotot', 'report.costs': 'kulut', 'report.result': 'tulos',
    'report.undistributedResult': 'jakamaton tulos', 'report.totalLiabilities': 'vastattavaa yhteensä',
    'report.netResult': 'nettotulos', 'report.profitAndLoss': 'TULOSLASKELMA', 'report.pnlSheet': 'Tuloslaskelma',
    'vat.file.description': 'Alv-ilmoitus{period} — siirto tilille {account} ({direction})',
    'vat.settle.description': 'Alv-ilmoituksen maksu{period} — {account} (pyöristysero {amount})',
    'email.invoiceSubject': 'Lasku {number} — {company}',
    'email.invoiceBody': 'Hyvä asiakas,\n\nLiitteenä lasku {number}, yhteensä {gross} (sis. alv).\n\nYstävällisin terveisin,\n{company}',
    'email.reminderSubject': 'Maksukehotus — lasku {number}',
    'email.reminderBody': 'Hyvä {name},\n\nLaskusta {number} on maksamatta {outstanding} (eräpäivä {dueDate}).\n{transfer}\n\nYstävällisin terveisin,\n{company}',
    'email.reminderTransferIban': 'Pyydämme maksamaan summan tilille IBAN {iban} viitteellä {number}.',
    'email.reminderTransferPlain': 'Pyydämme maksamaan avoimen summan viitteellä {number}.',
    'invlist.number': 'Numero', 'invlist.type': 'Tyyppi', 'invlist.date': 'Päivä',
    'invlist.customer': 'Asiakas', 'invlist.total': 'Yhteensä', 'invlist.status': 'Tila',
    'invlist.days': 'Päivää', 'invlist.reminder': 'Muistutus',
    'reminder.none': 'ei muistutuksia {date}',
    'entry.paid': 'maksettu {date}: {amount} ({method})',
    'entry.outstanding': 'maksamatta: {amount}',
    'monthend.totals': 'Summat:  debet {debit} / kredit {credit} {state}',
    'yearend.plan': 'Suunnitelma: sulje {year} — tulos {amount}{extra}',
    'yearend.closed': '{year} suljettu — tulos {amount} (kirjaukset #{entries}, kirjattu)',
    'yearend.status': '{year}: {state} — tulos {amount}',
  },

  // --- Norwegian Bokmål (nb-NO) ---
  nb: {
    'pdf.invoice': 'FAKTURA', 'pdf.credit': 'KREDITTERING',
    'pdf.billedTo': 'Fakturert til', 'pdf.date': 'Dato', 'pdf.dueDate': 'Forfallsdato',
    'pdf.reference': 'Referanse', 'pdf.kvk': 'Org.nr.', 'pdf.btw': 'Mva',
    'pdf.description': 'Beskrivelse', 'pdf.qty': 'Antall', 'pdf.unit': 'Enhet', 'pdf.price': 'Pris',
    'pdf.vat': 'Mva', 'pdf.discount': 'Rabatt', 'pdf.amount': 'Beløp',
    'pdf.subtotal': 'Delsum ekskl. mva', 'pdf.vatOn': 'Mva på', 'pdf.vatTotal': 'Mva totalt',
    'pdf.total': 'Totalt', 'pdf.inclVat': 'inkl. mva',
    'pdf.footerPay': 'Vennligst overfør beløpet innen {term} til IBAN {iban} til konto for {name}.',
    'pdf.dueDateTerm': '{date}', 'pdf.defaultTerm': 'den avtalte fristen',
    'unit.h': 't.', 'unit.day': 'dag', 'unit.month': 'måned', 'unit.unit': 'stk.',
    'unit.session': 'sesjon', 'unit.km': 'km', 'unit.kg': 'kg', 'unit.project': 'prosjekt',
    'status.draft': 'kladd', 'status.posted': 'bokført', 'status.open': 'åpen',
    'status.closed': 'lukket', 'status.overdue': 'forfalt', 'status.paid': 'betalt',
    'dir.payable': 'skyldig', 'dir.receivable': 'til gode', 'dir.debit': 'debet', 'dir.credit': 'kredit',
    'report.revenue': 'inntekter', 'report.costs': 'kostnader', 'report.result': 'resultat',
    'report.undistributedResult': 'udisponert resultat', 'report.totalLiabilities': 'gjeld og egenkapital totalt',
    'report.netResult': 'nettoresultat', 'report.profitAndLoss': 'RESULTATREGNSKAP', 'report.pnlSheet': 'Resultatregnskap',
    'vat.file.description': 'Mva-melding{period} — overføring til {account} ({direction})',
    'vat.settle.description': 'Betaling av mva-melding{period} — {account} (avrundingsdifferanse {amount})',
    'email.invoiceSubject': 'Faktura {number} — {company}',
    'email.invoiceBody': 'Kjære kunde,\n\nVedlagt følger faktura {number} på totalt {gross} (inkl. mva).\n\nMed vennlig hilsen,\n{company}',
    'email.reminderSubject': 'Purre — faktura {number}',
    'email.reminderBody': 'Kjære {name},\n\nFaktura {number} har et utestående beløp på {outstanding} (forfalt {dueDate}).\n{transfer}\n\nMed vennlig hilsen,\n{company}',
    'email.reminderTransferIban': 'Vennligst overfør beløpet til IBAN {iban} med henvisning {number}.',
    'email.reminderTransferPlain': 'Vennligst overfør det utestående beløpet med henvisning {number}.',
    'invlist.number': 'Nummer', 'invlist.type': 'Type', 'invlist.date': 'Dato',
    'invlist.customer': 'Kunde', 'invlist.total': 'Totalt', 'invlist.status': 'Status',
    'invlist.days': 'Dager', 'invlist.reminder': 'Purring',
    'reminder.none': 'ingen purringer per {date}',
    'entry.paid': 'betalt {date}: {amount} ({method})',
    'entry.outstanding': 'utestående: {amount}',
    'monthend.totals': 'Totaler:  debet {debit} / kredit {credit} {state}',
    'yearend.plan': 'Plan: avslutt {year} — resultat {amount}{extra}',
    'yearend.closed': '{year} avsluttet — resultat {amount} (posteringer #{entries}, bokført)',
    'yearend.status': '{year}: {state} — resultat {amount}',
  },

  // --- Swedish (sv-SE) ---
  sv: {
    'pdf.invoice': 'FAKTURA', 'pdf.credit': 'KREDITFAKTURA',
    'pdf.billedTo': 'Fakturerad till', 'pdf.date': 'Datum', 'pdf.dueDate': 'Förfallodatum',
    'pdf.reference': 'Referens', 'pdf.kvk': 'Org.nr.', 'pdf.btw': 'Moms',
    'pdf.description': 'Beskrivning', 'pdf.qty': 'Antal', 'pdf.unit': 'Enhet', 'pdf.price': 'Pris',
    'pdf.vat': 'Moms', 'pdf.discount': 'Rabatt', 'pdf.amount': 'Belopp',
    'pdf.subtotal': 'Delsumma exkl. moms', 'pdf.vatOn': 'Moms på', 'pdf.vatTotal': 'Moms totalt',
    'pdf.total': 'Totalt', 'pdf.inclVat': 'inkl. moms',
    'pdf.footerPay': 'Vänligen överför beloppet inom {term} till IBAN {iban} till förmån för {name}.',
    'pdf.dueDateTerm': '{date}', 'pdf.defaultTerm': 'den överenskomna tiden',
    'unit.h': 't.', 'unit.day': 'dag', 'unit.month': 'månad', 'unit.unit': 'st.',
    'unit.session': 'session', 'unit.km': 'km', 'unit.kg': 'kg', 'unit.project': 'projekt',
    'status.draft': 'utkast', 'status.posted': 'bokförd', 'status.open': 'öppen',
    'status.closed': 'stängd', 'status.overdue': 'försenad', 'status.paid': 'betald',
    'dir.payable': 'att betala', 'dir.receivable': 'att få', 'dir.debit': 'debet', 'dir.credit': 'kredit',
    'report.revenue': 'intäkter', 'report.costs': 'kostnader', 'report.result': 'resultat',
    'report.undistributedResult': 'ofördelat resultat', 'report.totalLiabilities': 'totala skulder och eget kapital',
    'report.netResult': 'nettoresultat', 'report.profitAndLoss': 'RESULTATRÄKNING', 'report.pnlSheet': 'Resultaträkning',
    'vat.file.description': 'Momsdeklaration{period} — överföring till {account} ({direction})',
    'vat.settle.description': 'Betalning av momsdeklaration{period} — {account} (avrundningsskillnad {amount})',
    'email.invoiceSubject': 'Faktura {number} — {company}',
    'email.invoiceBody': 'Hej,\n\nBifogad faktura {number} uppgår till {gross} (inkl. moms).\n\nVänliga hälsningar,\n{company}',
    'email.reminderSubject': 'Betalningspåminnelse — faktura {number}',
    'email.reminderBody': 'Hej {name},\n\nFaktura {number} har ett utestående belopp på {outstanding} (förfallen {dueDate}).\n{transfer}\n\nVänliga hälsningar,\n{company}',
    'email.reminderTransferIban': 'Vänligen överför beloppet till IBAN {iban} med referens {number}.',
    'email.reminderTransferPlain': 'Vänligen överför det utestående beloppet med referens {number}.',
    'invlist.number': 'Nummer', 'invlist.type': 'Typ', 'invlist.date': 'Datum',
    'invlist.customer': 'Kund', 'invlist.total': 'Totalt', 'invlist.status': 'Status',
    'invlist.days': 'Dagar', 'invlist.reminder': 'Påminnelse',
    'reminder.none': 'inga påminnelser per {date}',
    'entry.paid': 'betald {date}: {amount} ({method})',
    'entry.outstanding': 'utestående: {amount}',
    'monthend.totals': 'Summor:  debet {debit} / kredit {credit} {state}',
    'yearend.plan': 'Plan: avsluta {year} — resultat {amount}{extra}',
    'yearend.closed': '{year} avslutat — resultat {amount} (verifikationer #{entries}, bokfört)',
    'yearend.status': '{year}: {state} — resultat {amount}',
  },
};

/** Resolve the active UI locale: --locale flag > BUKIO_LOCALE env > 'en'.
 *
 * UI text is English by default (Aug 14 decision: UI/JSON stay English;
 * localization is an explicit opt-in via --locale / BUKIO_LOCALE). The
 * company's stored locale drives DOCUMENTS (invoice.language), not the CLI
 * surface — a company locale of 'nl' (the migration default) must not flip
 * the whole UI back to Dutch. */
export function resolveLocale(ctx = {}, db = null) {
  ctx = ctx ?? {};
  if (ctx.locale) return ctx.locale;
  if (process.env.BUKIO_LOCALE) return process.env.BUKIO_LOCALE;
  return 'en';
}

/** Translate key with {param} interpolation; fallback: exact locale ->
 *  base language (nl-be -> nl, fr-lu -> fr, de-DE -> de, en-GB -> en) ->
 *  'en' -> the key itself. */
export function t(key, params = {}, locale = 'en') {
  const loc = String(locale || 'en').toLowerCase();
  const base = loc.includes('-') ? loc.split('-')[0] : null;
  const s0 = TABLES[loc]?.[key] ?? (base ? TABLES[base]?.[key] : undefined) ?? TABLES.en[key] ?? key;
  let s = s0;
  for (const [k, v] of Object.entries(params)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}

/** Legacy invoice-label API (was src/invoice/i18n.js) — label(k, 'nl'|'en'). */
export function label(key, language = 'nl') {
  return t(`pdf.${key}`, {}, language);
}

/** Legacy unit-label API (was src/invoice/i18n.js). */
export function unitLabel(code, language = 'nl') {
  const s = t(`unit.${code}`, {}, language);
  return s === `unit.${code}` ? (code ?? '') : s;
}

// Backwards-compatible exports for importers of src/invoice/i18n.js.
export const LABELS = {
  nl: Object.fromEntries(Object.entries(TABLES.nl).filter(([k]) => k.startsWith('pdf.')).map(([k, v]) => [k.slice(4), v])),
  en: Object.fromEntries(Object.entries(TABLES.en).filter(([k]) => k.startsWith('pdf.')).map(([k, v]) => [k.slice(4), v])),
};
export const UNITS = {
  h: { nl: TABLES.nl['unit.h'], en: TABLES.en['unit.h'] },
  day: { nl: TABLES.nl['unit.day'], en: TABLES.en['unit.day'] },
  month: { nl: TABLES.nl['unit.month'], en: TABLES.en['unit.month'] },
  unit: { nl: TABLES.nl['unit.unit'], en: TABLES.en['unit.unit'] },
  session: { nl: TABLES.nl['unit.session'], en: TABLES.en['unit.session'] },
  km: { nl: TABLES.nl['unit.km'], en: TABLES.en['unit.km'] },
  kg: { nl: TABLES.nl['unit.kg'], en: TABLES.en['unit.kg'] },
  project: { nl: TABLES.nl['unit.project'], en: TABLES.en['unit.project'] },
};
