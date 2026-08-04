// Default chart of accounts (Phase 0 starter).
// VAT-agnostic by design: no BTW accounts here — the VAT module (Phase 2)
// adds them when enabled. Full RGS taxonomy import lands in Phase 1.

export const DEFAULT_CHART = [
  { code: '1000', name: 'Kas', type: 'asset', normalBalance: 'debit' },
  { code: '1100', name: 'Bank', type: 'asset', normalBalance: 'debit' },
  { code: '1200', name: 'Debiteuren', type: 'asset', normalBalance: 'debit' },
  { code: '2000', name: 'Crediteuren', type: 'liability', normalBalance: 'credit' },
  { code: '2100', name: 'Overige schulden', type: 'liability', normalBalance: 'credit' },
  { code: '3000', name: 'Eigen vermogen', type: 'equity', normalBalance: 'credit' },
  { code: '4000', name: 'Inkoopwaarde', type: 'expense', normalBalance: 'debit' },
  { code: '4100', name: 'Huisvestingskosten', type: 'expense', normalBalance: 'debit' },
  { code: '4200', name: 'Autokosten', type: 'expense', normalBalance: 'debit' },
  { code: '4300', name: 'Kantoor- en algemene kosten', type: 'expense', normalBalance: 'debit' },
  { code: '4400', name: 'Personeelskosten', type: 'expense', normalBalance: 'debit' },
  { code: '4500', name: 'Financiële baten en lasten', type: 'expense', normalBalance: 'debit' },
  { code: '8000', name: 'Omzet', type: 'income', normalBalance: 'credit' },
  { code: '8100', name: 'Overige opbrengsten', type: 'income', normalBalance: 'credit' },
];
