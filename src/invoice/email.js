/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Invoice email delivery: render the PDF (Playwright), send via the
// zero-dependency SMTP client, audit the delivery. Config comes from env
// (BUKIO_SMTP_*); recipients default to the contact's email. Status is
// 'sent' from finalize onward — emailing records the delivery, it does not
// change the invoice status.
import { formatAmount } from '../core/money.js';
import { sendMail, smtpConfig, smtpValidate } from '../core/smtp.js';
import { invoiceToPdf } from './pdf.js';
import { getInvoice } from './index.js';
import { t } from '../i18n/index.js';
import { record } from '../audit/index.js';

export function emailError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

export function defaultSubject(lang, invoiceNumber, companyName) {
  return t('email.invoiceSubject', { number: invoiceNumber, company: companyName }, lang);
}

export function defaultBody(lang, invoiceNumber, gross, companyName) {
  return t('email.invoiceBody', { number: invoiceNumber, gross, company: companyName }, lang);
}

/**
 * Email a finalized invoice (PDF attached) to the contact (or --to).
 * dryRun validates config + payload and renders the PDF plan, but makes no
 * network call, records nothing and changes nothing.
 */
export async function emailInvoice(db, {
  id, to = null, subject = null, body = null, attachPdf = true,
  actor = 'human', dryRun = false,
}) {
  const invoice = getInvoice(db, id);
  if (!invoice) throw emailError('NOT_FOUND', `invoice ${id} does not exist`);
  if (!invoice.invoice_number) throw emailError('NOT_FINALIZED', 'finalize the invoice before emailing it');

  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  const companyName = company?.name ?? 'Bukio';
  // the email follows the invoice's document language (any i18n table; t()
  // falls back en -> key, so an unknown code still yields English)
  const lang = invoice.language ?? 'en';
  const recipient = to ?? invoice.contact?.email ?? null;
  if (!recipient) throw emailError('CONTACT_EMAIL_MISSING', 'the contact has no email address — pass --to');

  const gross = formatAmount(invoice.gross_cents);
  const finalSubject = subject ?? defaultSubject(lang, invoice.invoice_number, companyName);
  const finalBody = body ?? defaultBody(lang, invoice.invoice_number, gross, companyName);

  const cfg = smtpConfig();
  smtpValidate(cfg);

  let attachment = null;
  if (attachPdf) {
    const pdf = await invoiceToPdf(db, invoice);
    attachment = { filename: `${invoice.invoice_number}.pdf`, mime: 'application/pdf', dataBase64: pdf.data, pdfBytes: pdf.bytes };
  }

  if (dryRun) {
    return {
      action: 'invoice.email', invoice_id: id, invoice_number: invoice.invoice_number,
      to: recipient, subject: finalSubject, body: finalBody,
      // report the real PDF byte count, not the base64 string length
      attachment: attachPdf ? { filename: attachment.filename, mime: attachment.mime, bytes: attachment.pdfBytes } : null,
      dryRun: true,
    };
  }

  const result = await sendMail({ ...cfg, to: recipient, subject: finalSubject, text: finalBody, attachment });

  db.transaction(() => {
    record(db, {
      actor, action: 'invoice.email', command: 'invoice email',
      args: {
        invoice_id: id, invoice_number: invoice.invoice_number, to: recipient,
        subject: finalSubject, attachment: attachPdf ? attachment.filename : null,
        server: result.server,
      },
      outcome: 'ok',
    });
  })();

  return {
    id, invoice_number: invoice.invoice_number, to: recipient, subject: finalSubject,
    delivered: true, server: result.server,
  };
}
