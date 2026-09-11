// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// UBL 2.1 / Peppol BIS 3.0 (EN 16931) invoice XML export.

use crate::accounts::resolve_profile;
use crate::invoice::{compute_invoice_totals, format_qty, line_discount_cents};
use crate::money::BukioError;
use rusqlite::Connection;
use serde_json::Value;

fn ubl_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

fn esc(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\x00'..='\x08' | '\x0B' | '\x0C' | '\x0E'..='\x1F'))
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            _ => c.to_string(),
        })
        .collect()
}

fn money_amount(cents: i64) -> String {
    format!("{:.2}", cents as f64 / 100.0)
}

pub fn vat_category(code: &str) -> &str {
    match code {
        "R" | "RE" => "AE",
        "0" => "Z",
        "V" | "M" => "E",
        _ if !code.is_empty() => "S",
        _ => "E",
    }
}

const UNIT_CODE_MAP: &[(&str, &str)] = &[
    ("h", "HUR"),
    ("day", "DAY"),
    ("month", "MON"),
    ("unit", "C62"),
    ("session", "C62"),
    ("km", "KMT"),
    ("kg", "KGM"),
    ("project", "C62"),
];

fn unit_code(unit: &str) -> &str {
    UNIT_CODE_MAP
        .iter()
        .find(|(k, _)| *k == unit)
        .map(|(_, v)| *v)
        .unwrap_or("C62")
}

fn address_block(
    party_name: &str,
    p: &Value,
    tax_id: Option<&str>,
    scheme_id: &str,
    default_country: &str,
) -> String {
    let postal = p
        .get("postalCode")
        .or_else(|| p.get("postal_code"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let endpoint = p
        .get("registration_id")
        .and_then(|v| v.as_str())
        .map(|rid| {
            format!(
                "\n        <cbc:EndpointID schemeID=\"{scheme_id}\">{}</cbc:EndpointID>",
                esc(rid)
            )
        })
        .unwrap_or_default();
    let tax_scheme = tax_id.map(|tid| {
        format!("\n          <cac:PartyTaxScheme><cbc:CompanyID schemeID=\"VAT\">{}</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme>", esc(tid))
    }).unwrap_or_else(|| "\n          ".to_string());
    let reg_id = p
        .get("registration_id")
        .and_then(|v| v.as_str())
        .map(|rid| format!("\n            <cbc:CompanyID>{}</cbc:CompanyID>", esc(rid)))
        .unwrap_or_else(|| "\n            ".to_string());

    format!(
        "\n        <cac:Party>{endpoint}
          <cac:PartyName><cbc:Name>{}</cbc:Name></cac:PartyName>
          <cac:PostalAddress>
            <cbc:StreetName>{}</cbc:StreetName>
            <cbc:CityName>{}</cbc:CityName>
            <cbc:PostalZone>{}</cbc:PostalZone>
            <cac:Country><cbc:IdentificationCode>{}</cbc:IdentificationCode></cac:Country>
          </cac:PostalAddress>{tax_scheme}
          <cac:PartyLegalEntity>
            <cbc:RegistrationName>{}</cbc:RegistrationName>{reg_id}
          </cac:PartyLegalEntity>
        </cac:Party>",
        esc(party_name),
        esc(p.get("address").and_then(|v| v.as_str()).unwrap_or("")),
        esc(p.get("city").and_then(|v| v.as_str()).unwrap_or("")),
        esc(postal),
        esc(p
            .get("country")
            .and_then(|v| v.as_str())
            .unwrap_or(default_country)),
        esc(party_name)
    )
}

fn buyer_scheme_id<'a>(profile: &'a Value, contact: &Value) -> &'a str {
    let seller_scheme = profile
        .get("identifiers")
        .and_then(|i| i.get("peppolSchemeId"))
        .and_then(|v| v.as_str())
        .unwrap_or("9944");
    let buyer_country = contact.get("country").and_then(|v| v.as_str()).unwrap_or(
        profile
            .get("meta")
            .and_then(|m| m.get("country"))
            .and_then(|v| v.as_str())
            .unwrap_or("NL"),
    );
    let seller_country = profile
        .get("meta")
        .and_then(|m| m.get("country"))
        .and_then(|v| v.as_str())
        .unwrap_or("NL");
    if buyer_country.eq_ignore_ascii_case(seller_country) {
        return seller_scheme;
    }
    // Cross-border: the buyer's registration number was issued by the buyer's
    // own registry, so it carries that market's scheme (an NL KVK number on a
    // LU seller's invoice is still scheme 9944). A market without a profile
    // (IS) keeps the seller's scheme.
    crate::accounts::get_profile(buyer_country)
        .ok()
        .and_then(|p| p["identifiers"]["peppolSchemeId"].as_str())
        .unwrap_or(seller_scheme)
}

fn fmt_pct(bp: i64) -> String {
    // the JS writes (rateBp / 100).toFixed(2) — always two decimals, so a 21%
    // rate is "21.00" (EN16931 BT-152 is a decimal, not a bare integer)
    format!("{:.2}", bp as f64 / 100.0)
}

/// The JS prints the allowance percentage as toFixed(4) with trailing zeros
/// stripped, so a 10% discount is "10" — not "10.0000".
fn mult_factor(pct: f64) -> String {
    let formatted = format!("{pct:.4}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn invoice_to_ubl(db: &Connection, invoice: &Value) -> Result<String, BukioError> {
    let profile = resolve_profile(db)?;
    let eformat = profile
        .get("documents")
        .and_then(|d| d.get("eInvoicing"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if eformat != "peppol-bis-3.0" {
        return Err(ubl_error(
            "FORMAT_NOT_SUPPORTED",
            format!("e-invoicing format '{eformat}' has no builder"),
        ));
    }
    build_peppol_bis30(db, invoice, &profile)
}

fn build_peppol_bis30(
    db: &Connection,
    invoice: &Value,
    profile: &Value,
) -> Result<String, BukioError> {
    let company = db
        .query_row("SELECT * FROM company WHERE id = 1", [], |r| {
            Ok(serde_json::json!({
                "name": r.get::<_, Option<String>>(1)?,
                "registration_id": r.get::<_, Option<String>>(2)?,
                // column indices follow the company schema: 4 tax_id, 5 iban,
                // 11 address, 12 postal_code, 13 city (the old ones read
                // vat_module/logo instead — iban at 6 is an INTEGER, so the
                // whole seller block failed NO_COMPANY)
                "tax_id": r.get::<_, Option<String>>(4)?,
                "iban": r.get::<_, Option<String>>(5)?,
                "address": r.get::<_, Option<String>>(11)?,
                "city": r.get::<_, Option<String>>(13)?,
                "postal_code": r.get::<_, Option<String>>(12)?,
            }))
        })
        .map_err(|_| ubl_error("NO_COMPANY", "no company initialised"))?;

    let is_credit = invoice
        .get("invoiceType")
        .or_else(|| invoice.get("invoice_type"))
        .and_then(|v| v.as_str())
        == Some("credit");
    let type_code = if is_credit { "381" } else { "380" };
    let currency = invoice
        .get("currency")
        .and_then(|v| v.as_str())
        .unwrap_or("EUR");
    let contact = invoice.get("contact").unwrap_or(&Value::Null);
    let lines: Vec<Value> = invoice
        .get("lines")
        .and_then(|l| l.as_array())
        .map(|a| a.iter().cloned().collect())
        .unwrap_or_default();
    let discount_type = invoice
        .get("discountType")
        .or_else(|| invoice.get("discount_type"))
        .and_then(|v| v.as_str());
    let discount_value = invoice
        .get("discountValue")
        .or_else(|| invoice.get("discount_value"))
        .and_then(|v| v.as_i64());

    // Credit note billing reference
    let credit_billing_ref = if is_credit {
        invoice
            .get("creditForInvoiceId")
            .or_else(|| invoice.get("credit_for_invoice_id"))
            .and_then(|v| v.as_i64())
            .and_then(|fid| {
                db.query_row(
                    "SELECT invoice_number FROM invoices WHERE id = ?1",
                    [fid],
                    |r| r.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()
            })
    } else {
        None
    };

    // Compute totals
    let totals = compute_invoice_totals(&lines, discount_type, discount_value);
    let net_cents = totals["net_cents"].as_i64().unwrap_or(0);
    let vat_cents = totals["vat_cents"].as_i64().unwrap_or(0);
    let gross_cents = totals["gross_cents"].as_i64().unwrap_or(0);
    let discount_cents = totals["discount_cents"].as_i64().unwrap_or(0);
    let net_before_cents = totals
        .get("net_before_cents")
        .or_else(|| totals.get("netBeforeCents"))
        .and_then(|v| v.as_i64())
        .unwrap_or(net_cents + discount_cents);
    let groups = totals
        .get("groups")
        .and_then(|g| g.as_array())
        .cloned()
        .unwrap_or_default();

    // Tax subtotals
    let mut subtotal_map: Vec<(String, String, i64, i64, i64)> = Vec::new();
    for g in &groups {
        let discounted_net = g
            .get("discountedNet")
            .or_else(|| g.get("discounted_net"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if discounted_net == 0 {
            continue;
        }
        let code = g.get("code").and_then(|v| v.as_str()).unwrap_or("");
        let rate_bp = g
            .get("rateBp")
            .or_else(|| g.get("rate_bp"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cat = vat_category(code);
        let key = format!("{cat}|{rate_bp}");
        if !subtotal_map.iter().any(|e| e.0 == key) {
            subtotal_map.push((key.clone(), cat.to_string(), rate_bp, 0, 0));
        }
        let e = subtotal_map.iter_mut().find(|e| e.0 == key).unwrap();
        e.3 += discounted_net;
        e.4 += g.get("vat").and_then(|v| v.as_i64()).unwrap_or(0);
    }
    let standard_rate_bp = profile
        .get("tax")
        .and_then(|t| t.get("standardRateBp"))
        .and_then(|v| v.as_i64())
        .unwrap_or(2100);
    let tax_subtotals: String = subtotal_map.iter().map(|(_, cat, rate_bp, base, vat)| {
        let pct = if cat == "AE" && *rate_bp > 0 { fmt_pct(*rate_bp) } else if cat == "AE" { fmt_pct(standard_rate_bp) } else { fmt_pct(*rate_bp) };
        format!("\n      <cac:TaxSubtotal>\n        <cbc:TaxableAmount currencyID=\"{currency}\">{}</cbc:TaxableAmount>\n        <cbc:TaxAmount currencyID=\"{currency}\">{}</cbc:TaxAmount>\n        <cac:TaxCategory>\n          <cbc:ID>{cat}</cbc:ID>\n          <cbc:Percent>{pct}</cbc:Percent>\n          <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>\n        </cac:TaxCategory>\n      </cac:TaxSubtotal>", money_amount(*base), money_amount(*vat))
    }).collect();

    // Lines XML
    let seller_scheme = profile
        .get("identifiers")
        .and_then(|i| i.get("peppolSchemeId"))
        .and_then(|v| v.as_str())
        .unwrap_or("9944");
    let default_country = profile
        .get("meta")
        .and_then(|m| m.get("country"))
        .and_then(|v| v.as_str())
        .unwrap_or("NL");
    let line_tag = if is_credit {
        "CreditNoteLine"
    } else {
        "InvoiceLine"
    };
    let qty_tag = if is_credit {
        "CreditNoteLineQuantity"
    } else {
        "InvoicedQuantity"
    };

    let lines_xml: String = lines.iter().enumerate().map(|(i, l)| {
        let desc = l.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let qty = l.get("quantity").and_then(|v| v.as_i64()).unwrap_or(0);
        let unit_price = l.get("unitPriceCents").or_else(|| l.get("unit_price_cents")).and_then(|v| v.as_i64()).unwrap_or(0);
        let amount = l.get("amountCents").or_else(|| l.get("amount_cents")).and_then(|v| v.as_i64()).unwrap_or(0);
        let vat_code = l.get("vatCode").or_else(|| l.get("vat_code")).and_then(|v| v.as_str()).unwrap_or("");
        let vat_rate_bp = l.get("vatRateBp").or_else(|| l.get("vat_rate_bp")).and_then(|v| v.as_i64()).unwrap_or(0);
        let unit = l.get("unit").and_then(|v| v.as_str()).unwrap_or("unit");
        let cat = vat_category(vat_code);
        let pct = if cat == "AE" && vat_rate_bp <= 0 { fmt_pct(standard_rate_bp) } else { fmt_pct(vat_rate_bp) };
        let uc = unit_code(unit);
        let disc = line_discount_cents(l);
        let line_net = amount - disc;
        let allowance = if disc > 0 {
            let pct_val = if l.get("discountType").or_else(|| l.get("discount_type")).and_then(|v| v.as_str()) == Some("pct") {
                l.get("discountValue").or_else(|| l.get("discount_value")).and_then(|v| v.as_i64()).unwrap_or(0) as f64 / 100.0
            } else if amount > 0 { (disc as f64 / amount as f64) * 100.0 } else { 0.0 };
            format!("\n      <cac:AllowanceCharge>\n        <cbc:ChargeIndicator>false</cbc:ChargeIndicator>\n        <cbc:AllowanceChargeReasonCode>95</cbc:AllowanceChargeReasonCode>\n        <cbc:Amount currencyID=\"{currency}\">{}</cbc:Amount>\n        <cbc:BaseAmount currencyID=\"{currency}\">{}</cbc:BaseAmount>\n        <cbc:MultiplierFactorNumeric>{}</cbc:MultiplierFactorNumeric>\n      </cac:AllowanceCharge>", money_amount(disc), money_amount(amount), mult_factor(pct_val))
        } else { String::new() };

        format!("\n    <cac:{line_tag}>\n      <cbc:ID>{}</cbc:ID>\n      <cbc:{qty_tag} unitCode=\"{uc}\">{}</cbc:{qty_tag}>\n      <cbc:LineExtensionAmount currencyID=\"{currency}\">{}</cbc:LineExtensionAmount>{allowance}\n      <cac:Item>\n        <cbc:Name>{}</cbc:Name>\n        <cac:ClassifiedTaxCategory>\n          <cbc:ID>{cat}</cbc:ID>\n          <cbc:Percent>{pct}</cbc:Percent>\n          <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>\n        </cac:ClassifiedTaxCategory>\n      </cac:Item>\n      <cac:Price>\n        <cbc:PriceAmount currencyID=\"{currency}\">{}</cbc:PriceAmount>\n      </cac:Price>\n    </cac:{line_tag}>",
            i + 1, format_qty(qty), money_amount(line_net), esc(desc), money_amount(unit_price))
    }).collect();

    let buyer_tax = contact.get("vatId").or_else(|| contact.get("vat_id")).and_then(|v| v.as_str()).map(|tid| {
        format!("\n        <cac:PartyTaxScheme><cbc:CompanyID schemeID=\"VAT\">{}</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme>", esc(tid))
    }).unwrap_or_default();
    let line_ext_total: i64 = lines
        .iter()
        .map(|l| {
            let amt = l
                .get("amountCents")
                .or_else(|| l.get("amount_cents"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            amt - line_discount_cents(l)
        })
        .sum();
    let allowance_total_xml = if discount_cents > 0 {
        format!("\n    <cbc:AllowanceTotalAmount currencyID=\"{currency}\">{}</cbc:AllowanceTotalAmount>", money_amount(discount_cents))
    } else {
        String::new()
    };
    let due_date = invoice
        .get("dueDate")
        .or_else(|| invoice.get("due_date"))
        .and_then(|v| v.as_str());
    let reference = invoice.get("reference").and_then(|v| v.as_str());
    let notes = invoice.get("notes").and_then(|v| v.as_str());
    let inv_number = invoice
        .get("invoiceNumber")
        .or_else(|| invoice.get("invoice_number"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let inv_date = invoice.get("date").and_then(|v| v.as_str()).unwrap_or("");

    let root_tag = if is_credit { "CreditNote" } else { "Invoice" };
    let root_ns = if is_credit {
        "urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2"
    } else {
        "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"
    };
    let type_tag = if is_credit {
        "CreditNoteTypeCode"
    } else {
        "InvoiceTypeCode"
    };

    let seller_party = address_block(
        company["name"].as_str().unwrap_or(""),
        &company,
        company["tax_id"].as_str(),
        seller_scheme,
        default_country,
    );
    let buyer_endpoint = contact
        .get("kvk")
        .and_then(|v| v.as_str())
        .map(|kvk| {
            let scheme = buyer_scheme_id(profile, contact);
            format!(
                "\n      <cbc:EndpointID schemeID=\"{scheme}\">{}</cbc:EndpointID>",
                esc(kvk)
            )
        })
        .unwrap_or_else(|| "      ".to_string());
    let buyer_reg_id = contact
        .get("kvk")
        .and_then(|v| v.as_str())
        .map(|kvk| format!("\n        <cbc:CompanyID>{}</cbc:CompanyID>", esc(kvk)))
        .unwrap_or_else(|| "\n        ".to_string());

    let credit_billing = if is_credit && credit_billing_ref.is_some() {
        format!("\n  \n  <cac:BillingReference>\n    <cac:InvoiceDocumentReference>\n      <cbc:ID>{}</cbc:ID>\n    </cac:InvoiceDocumentReference>\n  </cac:BillingReference>", esc(credit_billing_ref.as_deref().unwrap_or("")))
    } else {
        "\n  ".to_string()
    };
    let payment_means = company["iban"].as_str().map(|iban| {
        format!("\n  \n  <cac:PaymentMeans>\n    <cbc:PaymentMeansCode>30</cbc:PaymentMeansCode>\n    <cac:PayeeFinancialAccount><cbc:ID>{}</cbc:ID></cac:PayeeFinancialAccount>\n  </cac:PaymentMeans>", esc(iban))
    }).unwrap_or_else(|| "\n  ".to_string());
    let payment_terms = due_date.map(|dd| {
        format!("\n  <cac:PaymentTerms><cbc:PaymentDueDate>{dd}</cbc:PaymentDueDate></cac:PaymentTerms>")
    }).unwrap_or_else(|| "\n  ".to_string());
    let doc_allowance = if discount_cents > 0 {
        let pct_val = if net_before_cents > 0 {
            (discount_cents as f64 / net_before_cents as f64) * 100.0
        } else {
            0.0
        };
        format!("\n  \n  <cac:AllowanceCharge>\n    <cbc:ChargeIndicator>false</cbc:ChargeIndicator>\n    <cbc:AllowanceChargeReasonCode>95</cbc:AllowanceChargeReasonCode>\n    <cbc:Amount currencyID=\"{currency}\">{}</cbc:Amount>\n    <cbc:BaseAmount currencyID=\"{currency}\">{}</cbc:BaseAmount>\n    <cbc:MultiplierFactorNumeric>{}</cbc:MultiplierFactorNumeric>\n  </cac:AllowanceCharge>", money_amount(discount_cents), money_amount(net_before_cents), mult_factor(pct_val))
    } else {
        "\n  ".to_string()
    };
    let due_xml = due_date
        .map(|dd| format!("\n  <cbc:DueDate>{dd}</cbc:DueDate>"))
        .unwrap_or_else(|| "\n  ".to_string());
    let note_xml = notes
        .map(|n| format!("\n  <cbc:Note>{}</cbc:Note>", esc(n)))
        .unwrap_or_else(|| "\n  ".to_string());
    let ref_xml = reference
        .map(|r| format!("\n  <cbc:BuyerReference>{}</cbc:BuyerReference>", esc(r)))
        .unwrap_or_else(|| "\n  ".to_string());

    let xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<{root_tag} xmlns=\"{root_ns}\"\n         xmlns:cac=\"urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2\"\n         xmlns:cbc=\"urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2\">\n  <cbc:CustomizationID>urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0</cbc:CustomizationID>\n  <cbc:ProfileID>urn:fdc:peppol.eu:2017:poacc:billing:01:1.0</cbc:ProfileID>\n  <cbc:ID>{}</cbc:ID>\n  <cbc:IssueDate>{inv_date}</cbc:IssueDate>{due_xml}\n  <cbc:{type_tag}>{type_code}</cbc:{type_tag}>{note_xml}\n  <cbc:DocumentCurrencyCode>{currency}</cbc:DocumentCurrencyCode>{ref_xml}{credit_billing}\n  <cac:AccountingSupplierParty>{seller_party}</cac:AccountingSupplierParty>\n  <cac:AccountingCustomerParty>\n    <cac:Party>{buyer_endpoint}
      <cac:PartyName><cbc:Name>{}</cbc:Name></cac:PartyName>\n      <cac:PostalAddress>\n        <cbc:StreetName>{}</cbc:StreetName>\n        <cbc:CityName>{}</cbc:CityName>\n        <cbc:PostalZone>{}</cbc:PostalZone>\n        <cac:Country><cbc:IdentificationCode>{}</cbc:IdentificationCode></cac:Country>\n      </cac:PostalAddress>{buyer_tax}\n      <cac:PartyLegalEntity>\n        <cbc:RegistrationName>{}</cbc:RegistrationName>{buyer_reg_id}\n      </cac:PartyLegalEntity>\n    </cac:Party>\n  </cac:AccountingCustomerParty>{payment_means}{payment_terms}{doc_allowance}\n  <cac:TaxTotal>\n    <cbc:TaxAmount currencyID=\"{currency}\">{}</cbc:TaxAmount>{tax_subtotals}\n  </cac:TaxTotal>\n  <cac:LegalMonetaryTotal>\n    <cbc:LineExtensionAmount currencyID=\"{currency}\">{}</cbc:LineExtensionAmount>\n    <cbc:TaxExclusiveAmount currencyID=\"{currency}\">{}</cbc:TaxExclusiveAmount>\n    <cbc:TaxInclusiveAmount currencyID=\"{currency}\">{}</cbc:TaxInclusiveAmount>{allowance_total_xml}\n    <cbc:PayableAmount currencyID=\"{currency}\">{}</cbc:PayableAmount>\n  </cac:LegalMonetaryTotal>{lines_xml}\n</{root_tag}>",
        esc(inv_number),
        esc(contact.get("name").and_then(|v| v.as_str()).unwrap_or("")),
        esc(contact.get("address").and_then(|v| v.as_str()).unwrap_or("")),
        esc(contact.get("city").and_then(|v| v.as_str()).unwrap_or("")),
        esc(contact.get("postalCode").or_else(|| contact.get("postal_code")).and_then(|v| v.as_str()).unwrap_or("")),
        esc(contact.get("country").and_then(|v| v.as_str()).unwrap_or(default_country)),
        esc(contact.get("name").and_then(|v| v.as_str()).unwrap_or("")),
        money_amount(vat_cents),
        money_amount(line_ext_total),
        money_amount(net_cents),
        money_amount(gross_cents),
        money_amount(gross_cents),
    );

    Ok(xml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vat_categories() {
        assert_eq!(vat_category("21"), "S");
        assert_eq!(vat_category("0"), "Z");
        assert_eq!(vat_category("R"), "AE");
        assert_eq!(vat_category("RE"), "AE");
        assert_eq!(vat_category("V"), "E");
        assert_eq!(vat_category("M"), "E");
        assert_eq!(vat_category(""), "E");
    }

    #[test]
    fn unit_codes() {
        assert_eq!(unit_code("h"), "HUR");
        assert_eq!(unit_code("unit"), "C62");
        assert_eq!(unit_code("unknown"), "C62");
    }
}
