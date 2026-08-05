//! Default chart of accounts (Phase 1 — expanded, RGS-mapped).
//! VAT-agnostic by design: no BTW accounts here — the VAT module adds them
//! when enabled. Same data as the Node version (src/core/chart.js).

/// One account seed row (code, name, type, normal balance, RGS code).
#[derive(Debug, Clone, Copy)]
pub struct AccountSeed {
    pub code: &'static str,
    pub name: &'static str,
    pub account_type: &'static str,
    pub normal_balance: &'static str,
    pub rgs_code: &'static str,
}

pub const DEFAULT_CHART: &[AccountSeed] = &[
    // Activa
    AccountSeed { code: "1000", name: "Kas", account_type: "asset", normal_balance: "debit", rgs_code: "BLIM.10" },
    AccountSeed { code: "1100", name: "Bank", account_type: "asset", normal_balance: "debit", rgs_code: "BLIM.10" },
    AccountSeed { code: "1200", name: "Debiteuren", account_type: "asset", normal_balance: "debit", rgs_code: "BVOR.11" },
    AccountSeed { code: "1400", name: "Voorraad", account_type: "asset", normal_balance: "debit", rgs_code: "BVRD.30" },
    AccountSeed { code: "1600", name: "Overige vorderingen", account_type: "asset", normal_balance: "debit", rgs_code: "BVOR.11" },
    AccountSeed { code: "1700", name: "Vooruitbetaalde kosten", account_type: "asset", normal_balance: "debit", rgs_code: "BVOR.11" },
    AccountSeed { code: "1800", name: "Materiële vaste activa", account_type: "asset", normal_balance: "debit", rgs_code: "BMVA.02" },
    AccountSeed { code: "1850", name: "Vervoermiddelen", account_type: "asset", normal_balance: "debit", rgs_code: "BMVA.02" },
    // Passiva
    AccountSeed { code: "2000", name: "Crediteuren", account_type: "liability", normal_balance: "credit", rgs_code: "BSCH.12" },
    AccountSeed { code: "2100", name: "Overige schulden", account_type: "liability", normal_balance: "credit", rgs_code: "BSCH.12" },
    AccountSeed { code: "2300", name: "Vooruitontvangen bedragen", account_type: "liability", normal_balance: "credit", rgs_code: "BSCH.12" },
    AccountSeed { code: "2400", name: "Nog te betalen kosten", account_type: "liability", normal_balance: "credit", rgs_code: "BSCH.12" },
    AccountSeed { code: "2900", name: "Rekening-courant", account_type: "liability", normal_balance: "credit", rgs_code: "BSCH.12" },
    // Eigen vermogen
    AccountSeed { code: "3000", name: "Eigen vermogen", account_type: "equity", normal_balance: "credit", rgs_code: "BEIV.05" },
    // Kosten
    AccountSeed { code: "4000", name: "Inkoopwaarde", account_type: "expense", normal_balance: "debit", rgs_code: "WKPR.70" },
    AccountSeed { code: "4100", name: "Huisvestingskosten", account_type: "expense", normal_balance: "debit", rgs_code: "WBED.42" },
    AccountSeed { code: "4200", name: "Autokosten", account_type: "expense", normal_balance: "debit", rgs_code: "WBED.42" },
    AccountSeed { code: "4300", name: "Kantoor- en algemene kosten", account_type: "expense", normal_balance: "debit", rgs_code: "WBED.42" },
    AccountSeed { code: "4310", name: "Accountants- en administratiekosten", account_type: "expense", normal_balance: "debit", rgs_code: "WBED.42" },
    AccountSeed { code: "4320", name: "Verzekeringen", account_type: "expense", normal_balance: "debit", rgs_code: "WBED.42" },
    AccountSeed { code: "4330", name: "Telecommunicatie", account_type: "expense", normal_balance: "debit", rgs_code: "WBED.42" },
    AccountSeed { code: "4340", name: "Software en internetdiensten", account_type: "expense", normal_balance: "debit", rgs_code: "WBED.42" },
    AccountSeed { code: "4400", name: "Personeelskosten", account_type: "expense", normal_balance: "debit", rgs_code: "WPER.40" },
    AccountSeed { code: "4500", name: "Financiële baten en lasten", account_type: "expense", normal_balance: "debit", rgs_code: "WFBE.84" },
    AccountSeed { code: "4600", name: "Afschrijvingen", account_type: "expense", normal_balance: "debit", rgs_code: "WAFS.41" },
    AccountSeed { code: "4700", name: "Overige bedrijfskosten", account_type: "expense", normal_balance: "debit", rgs_code: "WBED.42" },
    // Opbrengsten
    AccountSeed { code: "8000", name: "Omzet", account_type: "income", normal_balance: "credit", rgs_code: "WOMZ.80" },
    AccountSeed { code: "8100", name: "Overige opbrengsten", account_type: "income", normal_balance: "credit", rgs_code: "WOVB.82" },
];

/// RGS hoofdgroep labels (official RGS nomenclature, Dutch).
pub const RGS_LABELS: &[(&str, &str)] = &[
    ("BMVA.02", "Materiële vaste activa"),
    ("BFVA.03", "Financiële vaste activa"),
    ("BVRD.30", "Voorraden"),
    ("BVOR.11", "Vorderingen"),
    ("BLIM.10", "Liquide middelen"),
    ("BEIV.05", "Eigen vermogen"),
    ("BVRZ.07", "Voorzieningen"),
    ("BLAS.08", "Langlopende schulden"),
    ("BSCH.12", "Kortlopende schulden"),
    ("WKPR.70", "Inkoopwaarde van de omzet"),
    ("WPER.40", "Personeelskosten"),
    ("WAFS.41", "Afschrijvingen"),
    ("WBED.42", "Overige bedrijfskosten"),
    ("WOMZ.80", "Omzet"),
    ("WOVB.82", "Overige bedrijfsopbrengsten"),
    ("WFBE.84", "Financiële baten en lasten"),
];

/// JS semantics: `RGS_LABELS[code] ?? code ?? 'Overig'`.
pub fn rgs_label(code: Option<&str>) -> String {
    match code {
        None => "Overig".to_string(),
        Some(c) => RGS_LABELS
            .iter()
            .find(|(k, _)| *k == c)
            .map(|(_, v)| v.to_string())
            .unwrap_or_else(|| c.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chart_has_28_accounts() {
        assert_eq!(DEFAULT_CHART.len(), 28);
        // codes are unique
        let mut codes: Vec<&str> = DEFAULT_CHART.iter().map(|a| a.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), 28);
    }

    #[test]
    fn rgs_labels() {
        assert_eq!(rgs_label(Some("BMVA.02")), "Materiële vaste activa");
        assert_eq!(rgs_label(Some("XX.99")), "XX.99");
        assert_eq!(rgs_label(None), "Overig");
    }
}
