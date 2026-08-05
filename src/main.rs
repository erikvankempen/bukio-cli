//! bukio-cli — CLI entrypoint (clap).
//!
//! Mirrors the Node commander surface. Output contract:
//!   success -> { "ok": true, "data": ... }   (stdout)
//!   failure -> { "ok": false, "error": { "code", "message" } }  (stdout, exit 1)
//! With --json only the JSON document is printed; otherwise human text.

use bukio_cli::bank;
use bukio_cli::recurring;
use bukio_cli::core::accounts;
use bukio_cli::core::chart::DEFAULT_CHART;
use bukio_cli::core::db;
use bukio_cli::core::entries::{self, parse_posting_specs};
use bukio_cli::core::money::format_amount;
use bukio_cli::fx;
use bukio_cli::invoice;
use bukio_cli::report;
use bukio_cli::vat;
use bukio_cli::AppError;
use clap::{Args, Parser, Subcommand};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;

fn default_db_path() -> String {
    if let Ok(p) = std::env::var("BUKIO_DB") {
        return p;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{home}/.bukio/bukio.db")
}

fn default_actor() -> String {
    std::env::var("BUKIO_ACTOR").unwrap_or_else(|_| "human".to_string())
}

fn today_iso() -> String {
    chrono::Utc::now().date_naive().format("%Y-%m-%d").to_string()
}

/// Node's JSON.stringify renders whole-number floats as integers (0.0 -> 0).
fn day_diff_json(d: f64) -> Value {
    if d.fract() == 0.0 {
        json!(d as i64)
    } else {
        json!(d)
    }
}

/// Resolved command context: clap gives us Option fields, we resolve the
/// env-aware defaults here so handlers can use plain values (Node semantics:
/// --db or BUKIO_DB or ~/.bukio/bukio.db; --actor or BUKIO_ACTOR or human).
struct Ctx {
    db: String,
    actor: String,
    json: bool,
    dry_run: bool,
}

#[derive(Parser)]
#[command(
    name = "bukio",
    version,
    about = "Agent-first, local-first double-entry bookkeeping for Dutch SMEs",
    disable_help_subcommand = true
)]
struct Cli {
    /// database file (env BUKIO_DB)
    #[arg(long, global = true)]
    db: Option<String>,
    /// acting entity (human or agent:<name>)
    #[arg(long, global = true)]
    actor: Option<String>,
    /// JSON output
    #[arg(long, global = true)]
    json: bool,
    /// show the plan without writing anything
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// initialise a company database (file, company row, default chart)
    Init(InitArgs),
    /// journal entries
    Entry {
        #[command(subcommand)]
        cmd: EntryCmd,
    },
    /// chart of accounts
    Account {
        #[command(subcommand)]
        cmd: AccountCmd,
    },
    /// VAT module
    Vat {
        #[command(subcommand)]
        cmd: VatCmd,
    },
    /// FX rates (vreemde valuta)
    Fx {
        #[command(subcommand)]
        cmd: FxCmd,
    },
    /// bank accounts, import and matching
    Bank {
        #[command(subcommand)]
        cmd: BankCmd,
    },
    /// recurring entries: templates, schedule, generation
    Recurring {
        #[command(subcommand)]
        cmd: RecurringCmd,
    },
    /// depreciation schedules (linear, remainder-adjusted final run)
    Depreciation {
        #[command(subcommand)]
        cmd: DepreciationCmd,
    },
    /// contacts (invoice counterparties)
    Contact {
        #[command(subcommand)]
        cmd: ContactCmd,
    },
    /// outgoing invoices
    Invoice {
        #[command(subcommand)]
        cmd: InvoiceCmd,
    },
    /// reports
    Report {
        #[command(subcommand)]
        cmd: ReportCmd,
    },
}

#[derive(Args)]
struct InitArgs {
    #[arg(long)]
    name: String,
    #[arg(long)]
    kvk: Option<String>,
    #[arg(long, default_value = "eenmanszaak")]
    legal_form: String,
    #[arg(long)]
    btw_id: Option<String>,
    #[arg(long)]
    iban: Option<String>,
    #[arg(long)]
    address: Option<String>,
    #[arg(long)]
    postal_code: Option<String>,
    #[arg(long)]
    city: Option<String>,
    /// enable the VAT module (Phase 2)
    #[arg(long, default_value = "off")]
    vat: String,
    /// small business scheme (KOR) — implies --vat off
    #[arg(long)]
    kor: bool,
    #[arg(long, default_value = "12-31")]
    fiscal_year_end: String,
}

#[derive(Subcommand)]
enum EntryCmd {
    /// create a journal entry (draft; --post to post immediately)
    Add(EntryAddArgs),    /// post a draft entry
    Post(EntryPostArgs),
    /// reverse a posted entry (creates a linked contra-entry)
    Reverse(EntryReverseArgs),
    /// list journal entries
    List(EntryListArgs),
    /// show one entry with its postings
    Show(EntryShowArgs),
}

#[derive(Args)]
struct EntryAddArgs {
    #[arg(long)]
    date: Option<String>,
    #[arg(long)]
    desc: String,
    #[arg(long = "postings", action = clap::ArgAction::Append)]
    postings: Vec<String>,
    /// manual|bank|invoice|agent
    #[arg(long, default_value = "manual")]
    source: String,
    #[arg(long)]
    source_ref: Option<String>,
    /// post the entry immediately (draft -> posted)
    #[arg(long)]
    post: bool,
}

#[derive(Args)]
struct EntryPostArgs {
    #[arg(long)]
    id: i64,
}

#[derive(Args)]
struct EntryReverseArgs {
    #[arg(long)]
    id: i64,
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Args)]
struct EntryListArgs {
    #[arg(long)]
    state: Option<String>,
    #[arg(long)]
    date_from: Option<String>,
    #[arg(long)]
    date_to: Option<String>,
    #[arg(long, default_value = "100")]
    limit: String,
}

#[derive(Args)]
struct EntryShowArgs {
    #[arg(long)]
    id: i64,
}

#[derive(Subcommand)]
enum AccountCmd {
    /// add an account
    Add(AccountAddArgs),
    /// list accounts
    List(AccountListArgs),
    /// show one account
    Show(AccountShowArgs),
    /// deactivate an account
    Deactivate(AccountCodeArgs),
    /// reactivate an account
    Reactivate(AccountCodeArgs),
    /// import a chart from CSV: code,name,type,normal_balance[,rgs_code]
    Import(AccountImportArgs),
}

#[derive(Args)]
struct AccountAddArgs {
    #[arg(long)]
    code: String,
    #[arg(long)]
    name: String,
    #[arg(long = "type")]
    account_type: String,
    #[arg(long)]
    normal_balance: String,
    #[arg(long)]
    rgs_code: Option<String>,
}

#[derive(Args)]
struct AccountListArgs {
    #[arg(long = "type")]
    account_type: Option<String>,
    #[arg(long)]
    include_inactive: bool,
}

#[derive(Args)]
struct AccountShowArgs {
    #[arg(long)]
    id: i64,
}

#[derive(Args)]
struct AccountCodeArgs {
    #[arg(long)]
    code: String,
}

#[derive(Args)]
struct AccountImportArgs {
    #[arg(long)]
    file: String,
}

#[derive(Subcommand)]
enum VatCmd {
    /// enable the VAT module (accounts 1500/2500 + codes)
    Enable,
    /// list VAT codes
    Codes,
    /// book a VAT-aware entry (e.g. 8000:-100.00@21)
    Book(VatBookArgs),
    /// OB-aangifte manual-filing readout (fields 1a-5d)
    Readout(VatReadoutArgs),
}

#[derive(Args)]
struct VatBookArgs {
    /// entry date
    #[arg(long)]
    date: String,
    /// description
    #[arg(long)]
    desc: String,
    /// posting specs CODE:AMOUNT[@VATCODE], repeatable or comma-separated
    #[arg(long, num_args = 1.., required = true)]
    postings: Vec<String>,
    /// manual|bank|invoice|agent
    #[arg(long, default_value = "manual")]
    source: String,
    /// source reference
    #[arg(long)]
    source_ref: Option<String>,
    /// postings are in this foreign currency; converted to EUR (needs a rate)
    #[arg(long)]
    currency: Option<String>,
    /// FX rate (1 EUR = n units); auto-looked-up on/before the date when omitted
    #[arg(long)]
    rate: Option<String>,
    /// post the entry immediately
    #[arg(long)]
    post: bool,
}

#[derive(Args)]
struct VatReadoutArgs {
    /// YYYY-Qn (quarter) or YYYY-MM (month)
    #[arg(long)]
    period: String,
    /// record that this period was filed manually
    #[arg(long)]
    mark_filed: bool,
}

#[derive(Subcommand)]
enum FxCmd {
    /// set an FX rate (1 EUR = n units) for a currency/date
    Set(FxSetArgs),
    /// list stored FX rates
    List(FxListArgs),
}

// --- bank -------------------------------------------------------------

#[derive(Args)]
struct BankAddArgs {
    /// IBAN
    #[arg(long)]
    iban: String,
    /// account name
    #[arg(long)]
    name: Option<String>,
    /// linked ledger account
    #[arg(long, default_value = "1100")]
    account_code: String,
}

#[derive(Args)]
struct BankImportArgs {
    /// CAMT.053 XML or CSV file
    #[arg(long)]
    file: String,
    /// IBAN of the bank account
    #[arg(long)]
    iban: String,
    /// camt|csv|auto (auto detects XML vs CSV)
    #[arg(long, default_value = "auto")]
    format: String,
    /// bank account name (if created)
    #[arg(long)]
    name: Option<String>,
    /// linked ledger account
    #[arg(long, default_value = "1100")]
    account_code: String,
}

#[derive(Args)]
struct BankTxListArgs {
    /// filter by account
    #[arg(long)]
    iban: Option<String>,
    /// unmatched|matched|ignored
    #[arg(long)]
    state: Option<String>,
    /// max rows
    #[arg(long, default_value = "200")]
    limit: String,
}

#[derive(Args)]
struct BankMatchAutoArgs {
    /// max |date difference| in days
    #[arg(long, default_value = "5")]
    window_days: String,
}

#[derive(Args)]
struct BankMatchLinkArgs {
    /// bank transaction id
    #[arg(long)]
    tx: String,
    /// entry id
    #[arg(long)]
    entry: String,
    /// exact|fuzzy|rule|manual|agent
    #[arg(long, default_value = "manual")]
    method: String,
}

#[derive(Args)]
struct BankMatchPostArgs {
    /// bank transaction id
    #[arg(long)]
    tx: String,
    /// counter account for the posting
    #[arg(long)]
    account: String,
}

#[derive(Args)]
struct BankTxIdArgs {
    /// bank transaction id
    #[arg(long)]
    tx: String,
}

#[derive(Subcommand)]
enum BankMatchCmd {
    /// auto-match unmatched transactions to posted entries
    Auto(BankMatchAutoArgs),
    /// list unmatched transactions with a proposed posting
    Suggest,
    /// link a transaction to an existing posted entry
    Link(BankMatchLinkArgs),
    /// post a new entry from an unmatched transaction (bank leg + counter leg)
    Post(BankMatchPostArgs),
}

#[derive(Subcommand)]
enum BankCmd {
    /// register a bank account
    Add(BankAddArgs),
    /// list bank accounts with balances and state counts
    List,
    /// import bank transactions from CAMT.053 XML or bank CSV (idempotent)
    Import(BankImportArgs),
    /// list bank transactions
    Transactions(BankTxListArgs),
    /// match transactions to entries
    Match {
        #[command(subcommand)]
        cmd: BankMatchCmd,
    },
    /// mark a transaction as ignored
    Ignore(BankTxIdArgs),
    /// re-open an ignored transaction
    Unignore(BankTxIdArgs),
}

#[derive(Args)]
struct FxSetArgs {
    /// ISO 4217 currency (3 letters)
    #[arg(long)]
    currency: String,
    /// rate date (yyyy-mm-dd)
    #[arg(long)]
    date: String,
    /// rate, e.g. 1.0875 (1 EUR = n units)
    #[arg(long)]
    rate: String,
}

#[derive(Args)]
struct FxListArgs {
    /// filter by currency
    #[arg(long)]
    currency: Option<String>,
    /// max rows
    #[arg(long, default_value = "50")]
    limit: String,
}

#[derive(Subcommand)]
enum ReportCmd {
    /// per-account debit/credit/net from posted entries
    TrialBalance(TbArgs),
    /// balance sheet as of a date (assets = liabilities + equity + result)
    Balans(BalansArgs),
    /// winst-en-verliesrekening for a period
    Pnl(PnlArgs),
    /// journal export (one row per posting) for a period
    Journal(PnlArgs),
}

#[derive(Args)]
struct ReportFmtArgs {
    /// json|csv|xlsx|human
    #[arg(long)]
    format: Option<String>,
    /// output file (csv/xlsx)
    #[arg(long)]
    out: Option<String>,
}

#[derive(Args)]
struct TbArgs {
    /// filter by year
    #[arg(long)]
    year: Option<String>,
    #[command(flatten)]
    fmt: ReportFmtArgs,
}

#[derive(Args)]
struct BalansArgs {
    /// balance date (yyyy-mm-dd)
    #[arg(long)]
    as_of: Option<String>,
    #[command(flatten)]
    fmt: ReportFmtArgs,
}

#[derive(Args)]
struct PnlArgs {
    /// fiscal year (overrides --from/--to)
    #[arg(long)]
    year: Option<String>,
    /// period start (inclusive)
    #[arg(long)]
    from: Option<String>,
    /// period end (inclusive)
    #[arg(long)]
    to: Option<String>,
    #[command(flatten)]
    fmt: ReportFmtArgs,
}

fn main() {
    let cli = Cli::parse();
    let ctx = Ctx {
        db: cli.db.unwrap_or_else(default_db_path),
        actor: cli.actor.unwrap_or_else(default_actor),
        json: cli.json,
        dry_run: cli.dry_run,
    };
    let result = match &cli.command {
        Command::Init(args) => cmd_init(&ctx, args),
        Command::Entry { cmd } => cmd_entry(&ctx, cmd),
        Command::Account { cmd } => cmd_account(&ctx, cmd),
        Command::Vat { cmd } => cmd_vat(&ctx, cmd),
        Command::Fx { cmd } => cmd_fx(&ctx, cmd),
        Command::Bank { cmd } => cmd_bank(&ctx, cmd),
        Command::Recurring { cmd } => cmd_recurring(&ctx, cmd),
        Command::Depreciation { cmd } => cmd_depreciation(&ctx, cmd),
        Command::Contact { cmd } => cmd_contact(&ctx, cmd),
        Command::Invoice { cmd } => cmd_invoice(&ctx, cmd),
        Command::Report { cmd } => cmd_report(&ctx, cmd),
    };
    if let Err(e) = result {
        fail(&ctx, &e);
    }
}

fn emit(ctx: &Ctx, data: Value, render: fn(&Ctx, &Value)) {
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&json!({ "ok": true, "data": data })).unwrap());
    } else {
        render(ctx, &data);
    }
}

fn fail(ctx: &Ctx, err: &AppError) -> ! {
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&json!({ "ok": false, "error": { "code": err.code(), "message": err.message } })).unwrap());
    } else {
        eprintln!("error [{}]: {}", err.code(), err.message);
    }
    std::process::exit(1);
}

fn ensure_db(ctx: &Ctx) -> bukio_cli::Result<Connection> {
    if !Path::new(&ctx.db).exists() {
        return Err(AppError::new("NO_DATABASE", format!("no database at {} — run 'bukio init' first", ctx.db)));
    }
    db::open_db(&ctx.db)
}

fn company_json(conn: &Connection) -> bukio_cli::Result<Value> {
    let row = conn.query_row(
        "SELECT name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module, kor_flag, fiscal_year_end FROM company WHERE id = 1",
        [],
        |r| {
            Ok(json!({
                "name": r.get::<_, String>(0)?,
                "kvk": r.get::<_, Option<String>>(1)?,
                "legal_form": r.get::<_, String>(2)?,
                "btw_id": r.get::<_, Option<String>>(3)?,
                "iban": r.get::<_, Option<String>>(4)?,
                "address": r.get::<_, Option<String>>(5)?,
                "postal_code": r.get::<_, Option<String>>(6)?,
                "city": r.get::<_, Option<String>>(7)?,
                "vat_module": r.get::<_, i64>(8)?,
                "kor_flag": r.get::<_, i64>(9)?,
                "fiscal_year_end": r.get::<_, String>(10)?,
            }))
        },
    )?;
    Ok(row)
}

// --- init -----------------------------------------------------------------

fn cmd_init(ctx: &Ctx, a: &InitArgs) -> bukio_cli::Result<()> {
    const LEGAL_FORMS: [&str; 6] = ["eenmanszaak", "vof", "bv", "nv", "stichting", "vereniging"];
    if !LEGAL_FORMS.contains(&a.legal_form.as_str()) {
        return Err(AppError::new("INVALID_LEGAL_FORM", format!("legal form '{}' must be one of {}", a.legal_form, LEGAL_FORMS.join("|"))));
    }
    let fye_ok = a.fiscal_year_end.len() == 5
        && a.fiscal_year_end.chars().enumerate().all(|(i, c)| if i == 2 { c == '-' } else { c.is_ascii_digit() });
    if !fye_ok {
        return Err(AppError::new("INVALID_FISCAL_YEAR_END", format!("fiscal year end '{}' must be mm-dd", a.fiscal_year_end)));
    }
    let vat_module = if a.kor { 0 } else if a.vat == "on" { 1 } else { 0 };
    let kor_flag = if a.kor { 1 } else { 0 };

    let company = json!({
        "name": a.name,
        "kvk": a.kvk,
        "legal_form": a.legal_form,
        "btw_id": a.btw_id,
        "iban": a.iban,
        "address": a.address,
        "postal_code": a.postal_code,
        "city": a.city,
        "vat_module": vat_module,
        "kor_flag": kor_flag,
        "fiscal_year_end": a.fiscal_year_end,
    });

    if ctx.dry_run {
        emit(ctx, json!({
            "action": "create company + seed default chart",
            "company": company,
            "db": &ctx.db,
            "db_exists": Path::new(&ctx.db).exists(),
            "chart": { "accounts": DEFAULT_CHART.len() + if vat_module == 1 { 2 } else { 0 } },
            "dryRun": true,
        }), render_init);
        return Ok(());
    }

    if Path::new(&ctx.db).exists() {
        let conn = db::open_db(&ctx.db)?;
        let has_company: i64 = conn.query_row("SELECT EXISTS(SELECT 1 FROM company)", [], |r| r.get(0))?;
        drop(conn);
        if has_company == 1 {
            return Err(AppError::new("ALREADY_INITIALISED", format!("database {} already has a company", ctx.db)));
        }
    }
    if let Some(parent) = Path::new(&ctx.db).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let conn = db::open_db(&ctx.db)?;
    conn.execute(
        "INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module, kor_flag, fiscal_year_end) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        rusqlite::params![
            a.name, a.kvk, a.legal_form, a.btw_id, a.iban,
            a.address, a.postal_code, a.city, vat_module, kor_flag, a.fiscal_year_end
        ],
    )?;
    let created = accounts::seed_default_chart(&conn)?;
    let mut vat_created = 0i64;
    if vat_module == 1 {
        let res = vat::enable_vat_module(&conn, &ctx.actor)?;
        vat_created = res.accounts.len() as i64;
    }
    bukio_cli::audit::record(&conn, &ctx.actor, "company.init", Some("init"), Some(&company), "ok", &[])?;
    let total = accounts::list_accounts(&conn, None, false)?.len() as i64;
    emit(ctx, json!({
        "company": company_json(&conn)?,
        "db": &ctx.db,
        "chart": { "accounts": total, "created": created + vat_created },
        "dryRun": false,
    }), render_init);
    Ok(())
}

// --- entry ----------------------------------------------------------------

fn serialize_entry(e: &entries::Entry) -> Value {
    serde_json::to_value(e).unwrap_or(Value::Null)
}

fn cmd_entry(ctx: &Ctx, cmd: &EntryCmd) -> bukio_cli::Result<()> {
    match cmd {
        EntryCmd::Add(a) => {
            let date = a.date.clone().unwrap_or_else(today_iso);
            let specs = parse_posting_specs(&a.postings)?;
            if ctx.dry_run {
                let db_exists = Path::new(&ctx.db).exists();
                let resolved: Option<Vec<Value>> = if db_exists {
                    let conn = db::open_db(&ctx.db)?;
                    let r = entries::resolve_postings(&conn, &specs)?.iter().map(|p| {
                        json!({
                            "code": p.code,
                            "amount_cents": p.amount_cents,
                            "amount": format_amount(p.amount_cents),
                            "fx_currency": p.fx_currency,
                            "fx_amount_cents": p.fx_amount_cents,
                        })
                    }).collect();
                    drop(conn);
                    Some(r)
                } else {
                    None
                };
                let sum: i64 = specs.iter().map(|p| p.amount_cents).sum();
                let postings = resolved.unwrap_or_else(|| {
                    specs.iter().map(|p| json!({
                        "code": p.code,
                        "amount_cents": p.amount_cents,
                        "amount": format_amount(p.amount_cents),
                    })).collect()
                });
                emit(ctx, json!({
                    "action": "create journal entry",
                    "date": &date,
                    "description": &a.desc,
                    "currency": Value::Null,
                    "postings": postings,
                    "sum_cents": sum,
                    "sum": format_amount(sum),
                    "state": "draft",
                    "post": a.post,
                    "account_validation": if db_exists { "ok" } else { "skipped (no database yet)" },
                    "dryRun": true,
                }), render_entry_plan);
                return Ok(());
            }
            let conn = ensure_db(ctx)?;
            let mut entry = entries::create_entry(&conn, &date, &a.desc, &specs, &a.source, a.source_ref.as_deref(), &ctx.actor)?;
            if a.post {
                entry = entries::post_entry(&conn, entry.meta.id, &ctx.actor)?;
            }
            emit(ctx, serialize_entry(&entry), render_entry);
            Ok(())
        }
        EntryCmd::Post(a) => {
            let conn = ensure_db(ctx)?;
            let entry = entries::get_entry(&conn, a.id)?;
            let Some(entry) = entry else {
                return Err(AppError::new("NOT_FOUND", format!("entry {} does not exist", a.id)));
            };
            if ctx.dry_run {
                emit(ctx, json!({
                    "action": "post entry",
                    "id": a.id,
                    "current_state": entry.meta.state,
                    "target_state": if entry.meta.state == "draft" { "posted" } else { "(no change)" },
                    "dryRun": true,
                }), render_post_plan);
                return Ok(());
            }
            let posted = entries::post_entry(&conn, a.id, &ctx.actor)?;
            emit(ctx, serialize_entry(&posted), render_entry);
            Ok(())
        }
        EntryCmd::Reverse(a) => {
            let conn = ensure_db(ctx)?;
            let entry = entries::get_entry(&conn, a.id)?;
            let Some(entry) = entry else {
                return Err(AppError::new("NOT_FOUND", format!("entry {} does not exist", a.id)));
            };
            if ctx.dry_run {
                let reversed_postings: Vec<Value> = entry.postings.iter().map(|p| json!({
                    "account_code": p.account_code,
                    "amount_cents": -p.amount_cents,
                    "amount": format_amount(-p.amount_cents),
                })).collect();
                emit(ctx, json!({
                    "action": "reverse entry (create linked contra-entry)",
                    "id": a.id,
                    "current_state": entry.meta.state,
                    "reversed_postings": reversed_postings,
                    "dryRun": true,
                }), render_reverse_plan);
                return Ok(());
            }
            let reversed = entries::reverse_entry(&conn, a.id, &ctx.actor, a.reason.as_deref())?;
            emit(ctx, serialize_entry(&reversed), render_entry);
            Ok(())
        }
        EntryCmd::List(a) => {
            let conn = ensure_db(ctx)?;
            let limit_n: i64 = a.limit.parse().unwrap_or(100);
            let list = entries::list_entries(&conn, a.state.as_deref(), a.date_from.as_deref(), a.date_to.as_deref(), limit_n)?;
            let data: Vec<Value> = list.iter().map(|e| json!({
                "id": e.id, "date": e.date, "description": e.description, "state": e.state,
                "source": e.source, "created_by": e.created_by,
            })).collect();
            emit(ctx, json!({ "entries": data }), render_entry_list);
            Ok(())
        }
        EntryCmd::Show(a) => {
            let conn = ensure_db(ctx)?;
            let entry = entries::get_entry(&conn, a.id)?;
            let Some(entry) = entry else {
                return Err(AppError::new("NOT_FOUND", format!("entry {} does not exist", a.id)));
            };
            emit(ctx, serialize_entry(&entry), render_entry);
            Ok(())
        }
    }
}

// --- account --------------------------------------------------------------

fn account_json(a: &accounts::Account) -> Value {
    serde_json::to_value(a).unwrap_or(Value::Null)
}

fn cmd_account(ctx: &Ctx, cmd: &AccountCmd) -> bukio_cli::Result<()> {
    match cmd {
        AccountCmd::Add(a) => {
            let conn = ensure_db(ctx)?;
            let acc = accounts::create_account(&conn, &a.code, &a.name, &a.account_type, &a.normal_balance, a.rgs_code.as_deref())?;
            emit(ctx, account_json(&acc), render_account);
            Ok(())
        }
        AccountCmd::List(a) => {
            let conn = ensure_db(ctx)?;
            let list = accounts::list_accounts(&conn, a.account_type.as_deref(), a.include_inactive)?;
            let data: Vec<Value> = list.iter().map(account_json).collect();
            emit(ctx, json!({ "accounts": data }), render_account_list);
            Ok(())
        }
        AccountCmd::Show(a) => {
            let conn = ensure_db(ctx)?;
            let acc = accounts::get_account(&conn, a.id)?;
            let Some(acc) = acc else {
                return Err(AppError::new("ACCOUNT_NOT_FOUND", format!("account {} does not exist", a.id)));
            };
            emit(ctx, account_json(&acc), render_account);
            Ok(())
        }
        AccountCmd::Deactivate(a) => {
            let conn = ensure_db(ctx)?;
            let acc = accounts::deactivate_account(&conn, &a.code)?;
            emit(ctx, account_json(&acc), render_account);
            Ok(())
        }
        AccountCmd::Reactivate(a) => {
            let conn = ensure_db(ctx)?;
            let acc = accounts::reactivate_account(&conn, &a.code)?;
            emit(ctx, account_json(&acc), render_account);
            Ok(())
        }
        AccountCmd::Import(a) => {
            let conn = ensure_db(ctx)?;
            let text = std::fs::read_to_string(&a.file)?;
            let result = accounts::import_chart_csv(&conn, &text)?;
            emit(ctx, serde_json::to_value(&result).unwrap(), render_import);
            Ok(())
        }
    }
}

// --- vat ------------------------------------------------------------------

fn cmd_vat(ctx: &Ctx, cmd: &VatCmd) -> bukio_cli::Result<()> {
    match cmd {
        VatCmd::Enable => {
            let conn = ensure_db(ctx)?;
            if ctx.dry_run {
                emit(
                    ctx,
                    json!({
                        "action": "enable VAT module",
                        "accounts": ["1500", "2500"],
                        "codes": ["21", "9", "0", "V", "R", "RE", "M", "P"],
                        "dryRun": true,
                    }),
                    render_vat_enable_plan,
                );
                return Ok(());
            }
            let res = vat::enable_vat_module(&conn, &ctx.actor)?;
            emit(ctx, serde_json::to_value(&res).unwrap(), render_vat_enable);
            Ok(())
        }
        VatCmd::Codes => {
            let conn = ensure_db(ctx)?;
            let codes = vat::list_vat_codes(&conn)?;
            emit(ctx, json!({ "codes": serde_json::to_value(&codes).unwrap() }), render_vat_codes);
            Ok(())
        }
        VatCmd::Book(a) => {
            let conn = ensure_db(ctx)?;
            let specs = vat::parse_vat_posting_specs(&a.postings)?;
            let converted: Vec<vat::VatSpec> = if let Some(currency) = &a.currency {
                let rate = fx::resolve_rate(&conn, Some(currency), a.rate.as_deref(), &a.date, &ctx.actor)?
                    .expect("currency given -> rate resolved");
                let raw: Vec<fx::RawSpec> = specs
                    .iter()
                    .map(|s| fx::RawSpec {
                        code: s.code.clone(),
                        amount_cents: s.amount_cents,
                        vat_code: s.vat_code.clone(),
                        fx_currency: s.fx_currency.clone(),
                        fx_amount_cents: s.fx_amount_cents,
                    })
                    .collect();
                let converted = fx::to_eur_postings(&raw, currency, rate);
                converted
                    .iter()
                    .map(|p| vat::VatSpec {
                        code: p.code.clone(),
                        amount_cents: p.amount_cents,
                        vat_code: p.vat_code.clone(),
                        fx_currency: p.fx_currency.clone(),
                        fx_amount_cents: p.fx_amount_cents,
                    })
                    .collect()
            } else {
                specs
            };
            if ctx.dry_run {
                let expanded = vat::expand_vat_postings(&conn, &converted)?;
                let postings: Vec<Value> = expanded
                    .iter()
                    .map(|p| {
                        let mut obj = serde_json::Map::new();
                        obj.insert("code".into(), json!(p.code));
                        obj.insert("amount_cents".into(), json!(p.amount_cents));
                        obj.insert("amount".into(), json!(format_amount(p.amount_cents)));
                        match &p.vat_code {
                            Some(vc) => { obj.insert("vat_code".into(), json!(vc)); }
                            None if !p.vat_leg => { obj.insert("vat_code".into(), Value::Null); }
                            None => {}
                        }
                        obj.insert("vat_amount".into(), json!(p.vat_amount_cents.map(format_amount)));
                        if let Some(cur) = &p.fx_currency {
                            obj.insert("fx_currency".into(), json!(cur));
                            obj.insert("fx_amount_cents".into(), json!(p.fx_amount_cents));
                        }
                        Value::Object(obj)
                    })
                    .collect();
                emit(
                    ctx,
                    json!({
                        "action": "book VAT entry (expanded)",
                        "date": a.date,
                        "description": a.desc,
                        "currency": a.currency,
                        "postings": postings,
                        "post": a.post,
                        "dryRun": true,
                    }),
                    render_vat_book_plan,
                );
                return Ok(());
            }
            let (entry, expanded) = vat::book_vat_entry(
                &conn,
                &a.date,
                &a.desc,
                &converted,
                &a.source,
                a.source_ref.as_deref(),
                &ctx.actor,
                a.post,
            )?;
            emit(
                ctx,
                json!({
                    "entry": vat_fmt_entry(&entry),
                    "expanded": expanded.iter().map(|p| {
                        let mut obj = serde_json::Map::new();
                        obj.insert("code".into(), json!(p.code));
                        match &p.vat_code {
                            Some(vc) => { obj.insert("vat_code".into(), json!(vc)); }
                            None if !p.vat_leg => { obj.insert("vat_code".into(), Value::Null); }
                            None => {}
                        }
                        Value::Object(obj)
                    }).collect::<Vec<_>>(),
                }),
                render_vat_book,
            );
            Ok(())
        }
        VatCmd::Readout(a) => {
            let conn = ensure_db(ctx)?;
            if a.mark_filed {
                let r = vat::mark_filed(&conn, &a.period, &ctx.actor)?;
                emit(
                    ctx,
                    json!({ "period": r.period, "from": r.from, "to": r.to, "status": r.status, "dryRun": false }),
                    render_mark_filed,
                );
                return Ok(());
            }
            let r = vat::ob_readout(&conn, &a.period)?;
            let fields: Value = r
                .fields
                .iter()
                .map(|(k, v)| (k.clone(), json!({ "cents": v, "amount": format_amount(*v) })))
                .collect::<serde_json::Map<String, Value>>()
                .into();
            emit(
                ctx,
                json!({
                    "period": r.period, "from": r.from, "to": r.to,
                    "fields": fields,
                    "to_pay_cents": r.to_pay_cents, "to_pay": r.to_pay,
                    "note": r.note,
                }),
                render_readout,
            );
            Ok(())
        }
    }
}

fn cmd_fx(ctx: &Ctx, cmd: &FxCmd) -> bukio_cli::Result<()> {
    match cmd {
        FxCmd::Set(a) => {
            let conn = ensure_db(ctx)?;
            let r = fx::set_fx_rate(&conn, &a.currency, &a.date, Some(&a.rate), "manual", &ctx.actor)?;
            emit(
                ctx,
                json!({ "rate": { "currency": r.currency, "date": r.date, "rate": r.rate, "rate_x10000": r.rate_x10000 } }),
                render_fx_set,
            );
            Ok(())
        }
        FxCmd::List(a) => {
            let conn = ensure_db(ctx)?;
            let limit: i64 = a.limit.parse().unwrap_or(50);
            let rates = fx::list_fx_rates(&conn, a.currency.as_deref(), limit)?;
            emit(ctx, json!({ "rates": serde_json::to_value(&rates).unwrap() }), render_fx_list);
            Ok(())
        }
    }
}

// --- bank handlers ------------------------------------------------------

fn parse_bank_file(file: &str, format: &str, iban: Option<&str>) -> bukio_cli::Result<Vec<bank::BankTx>> {
    let content = std::fs::read_to_string(file)
        .map_err(|e| AppError::new("IO_ERROR", format!("cannot read {file}: {e}")))?;
    let detected = if content.trim_start().starts_with('<') { "camt" } else { "csv" };
    let fmt = if format != "auto" { format } else { detected };
    match fmt {
        "camt" => bank::parse_camt053(&content),
        "csv" => bank::csv::parse_bank_csv(&content, iban),
        other => Err(AppError::new("INVALID_FORMAT", format!("unknown format '{other}' (use camt|csv|auto)"))),
    }
}

fn fmt_bank_tx(t: &bank::BankTxRow) -> Value {
    json!({
        "id": t.id, "date": t.date, "amount_cents": t.amount_cents, "amount": format_amount(t.amount_cents),
        "counterparty": t.counterparty, "description": t.description, "iban_counter": t.iban_counter,
        "iban": t.iban, "account_code": t.account_code, "state": t.state, "hash": t.hash,
    })
}

fn cmd_bank(ctx: &Ctx, cmd: &BankCmd) -> bukio_cli::Result<()> {
    match cmd {
        BankCmd::Add(a) => {
            let conn = ensure_db(ctx)?;
            let account = bank::get_or_create_bank_account(&conn, &a.iban, a.name.as_deref(), &a.account_code)?;
            emit(
                ctx,
                json!({ "bank_account": { "iban": account.iban, "name": account.name, "account_code": account.account_code, "id": account.id } }),
                render_bank_add,
            );
            Ok(())
        }
        BankCmd::List => {
            let conn = ensure_db(ctx)?;
            let accounts: Vec<Value> = bank::list_bank_accounts(&conn)?
                .iter()
                .map(|a| {
                    json!({
                        "iban": a.iban, "name": a.name, "account_code": a.account_code,
                        "transaction_count": a.transaction_count, "unmatched_count": a.unmatched_count,
                        "balance_cents": a.balance_cents, "balance": format_amount(a.balance_cents),
                    })
                })
                .collect();
            emit(ctx, json!({ "accounts": accounts }), render_bank_list);
            Ok(())
        }
        BankCmd::Import(a) => {
            let transactions = parse_bank_file(&a.file, &a.format, Some(&a.iban))?;
            let mut conn = ensure_db(ctx)?;
            if ctx.dry_run {
                let preview = bank::preview_import(&conn, &a.iban, &transactions)?;
                let shown: Vec<Value> = transactions
                    .iter()
                    .take(10)
                    .map(|t| {
                        json!({
                            "date": t.date, "amount_cents": t.amount_cents, "amount": format_amount(t.amount_cents),
                            "counterparty": t.counterparty, "description": t.description,
                            "iban_counter": t.iban_counter, "iban": t.iban,
                        })
                    })
                    .collect();
                emit(
                    ctx,
                    json!({
                        "action": "import bank transactions", "file": a.file,
                        "transactions": shown,
                        "iban": preview.iban, "imported": preview.imported,
                        "duplicates": preview.duplicates, "total": preview.total,
                        "dryRun": true,
                    }),
                    render_bank_import_plan,
                );
            } else {
                let result = bank::import_transactions(&mut conn, &a.iban, &transactions, a.name.as_deref(), &a.account_code, &ctx.actor)?;
                emit(
                    ctx,
                    json!({ "iban": result.iban, "imported": result.imported, "duplicates": result.duplicates, "total": result.total, "dryRun": false }),
                    render_bank_import,
                );
            }
            Ok(())
        }
        BankCmd::Transactions(a) => {
            let conn = ensure_db(ctx)?;
            let limit: i64 = a.limit.parse().unwrap_or(200);
            let rows = bank::list_transactions(&conn, a.state.as_deref(), a.iban.as_deref(), limit)?;
            let data: Vec<Value> = rows.iter().map(fmt_bank_tx).collect();
            emit(ctx, json!({ "transactions": data }), render_bank_tx_list);
            Ok(())
        }
        BankCmd::Match { cmd } => cmd_bank_match(ctx, cmd),
        BankCmd::Ignore(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.tx.parse().map_err(|_| AppError::new("INVALID_ID", format!("tx '{}' is not an id", a.tx)))?;
            let tx_row = bank::set_transaction_state(&conn, id, "ignored", &ctx.actor)?;
            emit(ctx, json!({ "transaction": fmt_bank_tx(&tx_row) }), render_bank_ignore);
            Ok(())
        }
        BankCmd::Unignore(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.tx.parse().map_err(|_| AppError::new("INVALID_ID", format!("tx '{}' is not an id", a.tx)))?;
            let tx_row = bank::set_transaction_state(&conn, id, "unmatched", &ctx.actor)?;
            emit(ctx, json!({ "transaction": fmt_bank_tx(&tx_row) }), render_bank_unignore);
            Ok(())
        }
    }
}

fn cmd_bank_match(ctx: &Ctx, cmd: &BankMatchCmd) -> bukio_cli::Result<()> {
    match cmd {
        BankMatchCmd::Auto(a) => {
            let mut conn = ensure_db(ctx)?;
            let window: i64 = a.window_days.parse().unwrap_or(5);
            let result = bank::auto_match(&mut conn, window, &ctx.actor, ctx.dry_run)?;
            let matched: Vec<Value> = result
                .matched
                .iter()
                .map(|m| {
                    let mut obj = serde_json::Map::new();
                    obj.insert("kind".into(), json!(m.kind));
                    obj.insert("tx_id".into(), json!(m.tx_id));
                    obj.insert("tx_date".into(), json!(m.tx_date));
                    obj.insert("amount_cents".into(), json!(m.amount_cents));
                    obj.insert("description".into(), json!(m.description));
                    obj.insert("counterparty".into(), json!(m.counterparty));
                    if let Some(i) = m.invoice_id {
                        obj.insert("invoice_id".into(), json!(i));
                        obj.insert("invoice_number".into(), json!(m.invoice_number));
                        obj.insert("contact_name".into(), json!(m.contact_name));
                    }
                    if let Some(e) = m.entry_id {
                        obj.insert("entry_id".into(), json!(e));
                        obj.insert("entry_date".into(), json!(m.entry_date));
                        obj.insert("day_diff".into(), json!(m.day_diff.map(day_diff_json)));
                    }
                    obj.insert("method".into(), json!(m.method));
                    obj.insert("confidence".into(), json!(m.confidence));
                    // Node appends amount last via { ...m, amount }
                    obj.insert("amount".into(), json!(format_amount(m.amount_cents)));
                    Value::Object(obj)
                })
                .collect();
            emit(
                ctx,
                json!({ "matched": matched, "unmatched_remaining": result.unmatched_remaining, "dryRun": ctx.dry_run }),
                render_bank_match_auto,
            );
            Ok(())
        }
        BankMatchCmd::Suggest => {
            let conn = ensure_db(ctx)?;
            let rows = bank::suggest_unmatched(&conn)?;
            let data: Vec<Value> = rows
                .iter()
                .map(|(t, suggested)| {
                    let mut v = fmt_bank_tx(t);
                    v.as_object_mut().unwrap().insert("suggested_account".into(), json!(suggested));
                    v
                })
                .collect();
            emit(ctx, json!({ "suggestions": data }), render_bank_suggest);
            Ok(())
        }
        BankMatchCmd::Link(a) => {
            let conn = ensure_db(ctx)?;
            let tx_id: i64 = a.tx.parse().map_err(|_| AppError::new("INVALID_ID", format!("tx '{}' is not an id", a.tx)))?;
            let entry_id: i64 = a.entry.parse().map_err(|_| AppError::new("INVALID_ID", format!("entry '{}' is not an id", a.entry)))?;
            let tx_row = bank::link_transaction(&conn, tx_id, entry_id, &a.method, None, &ctx.actor)?;
            emit(
                ctx,
                json!({ "transaction": fmt_bank_tx(&tx_row), "entry_id": entry_id }),
                render_bank_link,
            );
            Ok(())
        }
        BankMatchCmd::Post(a) => {
            let conn = ensure_db(ctx)?;
            let tx_id: i64 = a.tx.parse().map_err(|_| AppError::new("INVALID_ID", format!("tx '{}' is not an id", a.tx)))?;
            let tx_row = bank::get_transaction(&conn, tx_id)?.ok_or_else(|| {
                AppError::new("NOT_FOUND", format!("bank transaction {tx_id} does not exist"))
            })?;
            if ctx.dry_run {
                emit(
                    ctx,
                    json!({
                        "action": "post entry from bank transaction",
                        "tx": fmt_bank_tx(&tx_row),
                        "postings": [
                            { "code": tx_row.account_code, "amount_cents": tx_row.amount_cents, "amount": format_amount(tx_row.amount_cents) },
                            { "code": a.account, "amount_cents": -tx_row.amount_cents, "amount": format_amount(-tx_row.amount_cents) },
                        ],
                        "dryRun": true,
                    }),
                    render_bank_post_plan,
                );
            } else {
                let (_, entry) = bank::post_from_transaction(&conn, tx_id, &a.account, &ctx.actor, true)?;
                emit(
                    ctx,
                    json!({ "entry_id": entry.meta.id, "state": entry.meta.state }),
                    render_bank_post,
                );
            }
            Ok(())
        }
    }
}

/// Node vat.js fmtEntry: postings carry vat fields; `vat_code` is null when no
/// VAT code, omitted when there is one (JSON.stringify drops undefined).
fn vat_fmt_entry(entry: &entries::Entry) -> Value {
    json!({
        "id": entry.meta.id,
        "date": entry.meta.date,
        "description": entry.meta.description,
        "state": entry.meta.state,
        "postings": entry.postings.iter().map(|p| {
            let mut obj = serde_json::Map::new();
            obj.insert("account_code".into(), json!(p.account_code));
            obj.insert("account_name".into(), json!(p.account_name));
            obj.insert("amount_cents".into(), json!(p.amount_cents));
            obj.insert("amount".into(), json!(format_amount(p.amount_cents)));
            if p.vat_code_id.is_some() {
                obj.insert("vat_amount_cents".into(), json!(p.vat_amount_cents));
                obj.insert("vat_amount".into(), json!(p.vat_amount_cents.map(format_amount)));
            } else {
                obj.insert("vat_code".into(), Value::Null);
                obj.insert("vat_amount_cents".into(), Value::Null);
                obj.insert("vat_amount".into(), Value::Null);
            }
            Value::Object(obj)
        }).collect::<Vec<_>>(),
    })
}

// --- recurring ------------------------------------------------------------

#[derive(Subcommand)]
enum RecurringCmd {
    /// create a recurring template (entry postings or subscription invoices)
    Add(RecurringAddArgs),
    /// list recurring templates
    List(RecurringListArgs),
    /// show one template with its postings
    Show(RecurringIdArgs),
    /// pause a template
    Pause(RecurringIdArgs),
    /// resume a paused template
    Resume(RecurringIdArgs),
    /// show what is due (no writes)
    Preview(RecurringRunArgs),
    /// generate all due entries (idempotent, backfills missed periods)
    Run(RecurringRunArgs),
}

#[derive(Args)]
struct RecurringAddArgs {
    /// template name (also the entry description prefix)
    #[arg(long)]
    name: String,
    /// template kind
    #[arg(long, default_value = "entry")]
    kind: String,
    /// contact id (required for --kind invoice)
    #[arg(long)]
    contact: Option<String>,
    /// invoice line specs "[QTYx] DESC @ PRICE [@ VATCODE]" (invoice kind)
    #[arg(long)]
    lines: Option<String>,
    /// posting specs, comma-separated (entry kind)
    #[arg(long)]
    postings: Option<String>,
    /// one of monthly, quarterly, yearly
    #[arg(long)]
    frequency: String,
    /// first run date
    #[arg(long)]
    start: String,
    /// day of period to book on (1-28)
    #[arg(long, default_value = "1")]
    day: String,
    /// last run date
    #[arg(long)]
    end: Option<String>,
    /// maximum number of runs
    #[arg(long)]
    runs: Option<String>,
    /// payment term for invoice templates (days)
    #[arg(long, default_value = "30")]
    due_days: String,
    /// template description
    #[arg(long)]
    desc: Option<String>,
    /// accrual pattern: reverse the previous generated entry on each run
    #[arg(long)]
    reverse_previous: bool,
}

#[derive(Args)]
struct RecurringListArgs {
    /// active|paused|completed|all
    #[arg(long, default_value = "active")]
    status: String,
}

#[derive(Args)]
struct RecurringIdArgs {
    /// template id
    #[arg(long)]
    id: String,
}

#[derive(Args)]
struct RecurringRunArgs {
    /// reference date (default today)
    #[arg(long)]
    as_of: Option<String>,
    /// only this template
    #[arg(long)]
    template: Option<String>,
}

#[derive(Subcommand)]
enum DepreciationCmd {
    /// create a monthly depreciation template for an asset
    Add(DepreciationAddArgs),
}

#[derive(Args)]
struct DepreciationAddArgs {
    /// asset name
    #[arg(long)]
    name: String,
    /// purchase cost (e.g. 5370.00)
    #[arg(long)]
    cost: String,
    /// useful life in months
    #[arg(long)]
    life_months: String,
    /// first depreciation month
    #[arg(long)]
    start: String,
    /// asset account
    #[arg(long, default_value = "1800")]
    asset: String,
    /// depreciation expense account
    #[arg(long, default_value = "4600")]
    expense: String,
    /// residual value at end of life
    #[arg(long, default_value = "0")]
    residual: String,
    /// template description
    #[arg(long)]
    desc: Option<String>,
}

// --- recurring handlers ---------------------------------------------------

fn fmt_postings(postings: &[recurring::TemplatePosting]) -> Vec<Value> {
    postings
        .iter()
        .map(|p| {
            json!({
                "code": p.code,
                "amount_cents": p.amount_cents,
                "amount": format_amount(p.amount_cents),
                "vat_code": p.vat_code,
            })
        })
        .collect()
}

fn fmt_template(t: &recurring::Template) -> Value {
    json!({
        "id": t.id, "name": t.name, "description": t.description,
        "kind": t.kind, "contact_id": t.contact_id,
        "invoice_lines": t.invoice_lines,
        "frequency": t.frequency, "day_of_period": t.day_of_period,
        "start_date": t.start_date, "end_date": t.end_date, "runs": t.runs,
        "status": t.status, "next_run_date": t.next_run_date, "last_run_date": t.last_run_date,
        "runs_done": t.runs_done, "reverse_previous": t.reverse_previous,
        "vat_aware": t.vat_aware,
        "postings": fmt_postings(&t.postings),
        "final_postings": t.final_postings.as_ref().map(|fp| fmt_postings(fp)),
    })
}

fn cmd_recurring(ctx: &Ctx, cmd: &RecurringCmd) -> bukio_cli::Result<()> {
    match cmd {
        RecurringCmd::Add(a) => {
            let conn = ensure_db(ctx)?;
            let day: i64 = a.day.parse().unwrap_or(1);
            let runs: Option<i64> = a.runs.as_deref().map(|r| r.parse().unwrap_or(0));
            let contact: Option<i64> = a.contact.as_deref().map(|c| c.parse().unwrap_or(0));
            let postings: Vec<String> = a
                .postings
                .as_deref()
                .map(|p| p.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default();
            let lines: Option<Vec<String>> = a
                .lines
                .as_deref()
                .map(|l| l.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
            if ctx.dry_run {
                emit(
                    ctx,
                    json!({
                        "action": "create recurring template",
                        "kind": a.kind, "name": a.name, "frequency": a.frequency,
                        "day_of_period": day, "start_date": a.start,
                        "end_date": a.end, "runs": runs,
                        "reverse_previous": a.reverse_previous,
                        "contact": contact, "postings": a.postings, "lines": a.lines,
                        "dryRun": true,
                    }),
                    render_recurring_add_plan,
                );
                return Ok(());
            }
            let tpl = recurring::create_template(
                &conn,
                &a.name,
                a.desc.as_deref(),
                &a.frequency,
                day,
                &a.start,
                a.end.as_deref(),
                runs,
                &postings,
                a.reverse_previous,
                &ctx.actor,
                &a.kind,
                contact,
                lines.as_deref(),
                Some(a.due_days.parse().unwrap_or(30)),
            )?;
            emit(ctx, json!({ "template": fmt_template(&tpl), "dryRun": false }), render_recurring_add);
            Ok(())
        }
        RecurringCmd::List(a) => {
            let conn = ensure_db(ctx)?;
            let rows = recurring::list_templates(&conn, &a.status)?;
            let data: Vec<Value> = rows.iter().map(fmt_template).collect();
            emit(ctx, json!({ "templates": data }), render_recurring_list);
            Ok(())
        }
        RecurringCmd::Show(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.id.parse().map_err(|_| AppError::new("INVALID_ID", format!("id '{}' is not a number", a.id)))?;
            let tpl = recurring::get_template(&conn, id)?.ok_or_else(|| {
                AppError::new("NOT_FOUND", format!("recurring template {id} does not exist"))
            })?;
            emit(ctx, json!({ "template": fmt_template(&tpl) }), render_recurring_show);
            Ok(())
        }
        RecurringCmd::Pause(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.id.parse().map_err(|_| AppError::new("INVALID_ID", format!("id '{}' is not a number", a.id)))?;
            let tpl = recurring::set_template_status(&conn, id, "paused", &ctx.actor)?;
            emit(ctx, json!({ "template": fmt_template(&tpl) }), |_c, d| println!("paused template #{}", d["template"]["id"]));
            Ok(())
        }
        RecurringCmd::Resume(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.id.parse().map_err(|_| AppError::new("INVALID_ID", format!("id '{}' is not a number", a.id)))?;
            let tpl = recurring::set_template_status(&conn, id, "active", &ctx.actor)?;
            emit(ctx, json!({ "template": fmt_template(&tpl) }), |_c, d| println!("resumed template #{}", d["template"]["id"]));
            Ok(())
        }
        RecurringCmd::Preview(a) => {
            let mut conn = ensure_db(ctx)?;
            let tid: Option<i64> = a.template.as_deref().map(|t| t.parse().unwrap_or(0));
            let plan = recurring::preview_due(&mut conn, a.as_of.as_deref(), tid)?;
            emit(ctx, plan, render_recurring_preview);
            Ok(())
        }
        RecurringCmd::Run(a) => {
            let mut conn = ensure_db(ctx)?;
            let tid: Option<i64> = a.template.as_deref().map(|t| t.parse().unwrap_or(0));
            let result = recurring::run_due(&mut conn, a.as_of.as_deref(), tid, &ctx.actor, ctx.dry_run, 120)?;
            // CLI shape (mirrors Node): {template_id, name, ok, error ?? null,
            // runs: [{kind, entries}]} — kind omitted for real runs, 'entry'
            // for dry-run entry sims.
            let mapped: Vec<Value> = result["templates"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|t| {
                            let runs: Vec<Value> = t["runs"]
                                .as_array()
                                .map(|runs| {
                                    runs.iter()
                                        .map(|r| {
                                            if r.get("entries").is_some() {
                                                json!({ "entries": r["entries"] })
                                            } else {
                                                let kind = if r["kind"] == "invoice" {
                                                    "invoice"
                                                } else {
                                                    "entry"
                                                };
                                                json!({ "kind": kind, "entries": [r["entry"]] })
                                            }
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            json!({
                                "template_id": t["template_id"],
                                "name": t["name"],
                                "ok": t["ok"],
                                "error": t.get("error").cloned().unwrap_or(Value::Null),
                                "runs": runs,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            emit(
                ctx,
                json!({ "as_of": result["as_of"], "dry_run": result["dry_run"], "templates": mapped }),
                render_recurring_run,
            );
            Ok(())
        }
    }
}

fn cmd_depreciation(ctx: &Ctx, cmd: &DepreciationCmd) -> bukio_cli::Result<()> {
    match cmd {
        DepreciationCmd::Add(a) => {
            let conn = ensure_db(ctx)?;
            let cost_cents = (a.cost.parse::<f64>().unwrap_or(0.0) * 100.0).round() as i64;
            let residual_cents = (a.residual.parse::<f64>().unwrap_or(0.0) * 100.0).round() as i64;
            let life_months: i64 = a.life_months.parse().unwrap_or(0);
            if ctx.dry_run {
                let monthly = ((cost_cents - residual_cents) as f64 / life_months as f64).round() as i64;
                let final_cents = (cost_cents - residual_cents) - monthly * (life_months - 1);
                emit(
                    ctx,
                    json!({
                        "action": "create depreciation template",
                        "name": a.name, "asset": a.asset, "expense": a.expense,
                        "cost_cents": cost_cents, "residual_cents": residual_cents,
                        "life_months": life_months,
                        "monthly_cents": monthly, "final_cents": final_cents,
                        "monthly": format_amount(monthly), "final": format_amount(final_cents),
                        "dryRun": true,
                    }),
                    render_depreciation_add_plan,
                );
                return Ok(());
            }
            let result = recurring::build_depreciation_template(
                &conn,
                &a.name,
                &a.asset,
                &a.expense,
                cost_cents,
                residual_cents,
                life_months,
                &a.start,
                a.desc.as_deref(),
                &ctx.actor,
            )?;
            emit(
                ctx,
                json!({
                    "template": fmt_template(&serde_json::from_value::<recurring::Template>(result["template"].clone()).unwrap()),
                    "monthly": result["monthly"], "final": result["final"],
                    "total": format_amount(result["total_cents"].as_i64().unwrap_or(0)),
                }),
                render_depreciation_add,
            );
            Ok(())
        }
    }
}

fn cmd_contact(ctx: &Ctx, cmd: &ContactCmd) -> bukio_cli::Result<()> {
    match cmd {
        ContactCmd::Add(a) => {
            let conn = ensure_db(ctx)?;
            let contact = invoice::create_contact(
                &conn,
                &a.name,
                a.address.as_deref(),
                a.postal_code.as_deref(),
                a.city.as_deref(),
                &a.country,
                a.email.as_deref(),
                a.vat_id.as_deref(),
                a.kvk.as_deref(),
                &ctx.actor,
            )?;
            emit(ctx, json!({ "contact": contact_json(&contact) }), render_contact);
            Ok(())
        }
        ContactCmd::List => {
            let conn = ensure_db(ctx)?;
            let contacts = invoice::list_contacts(&conn)?;
            let data: Vec<Value> = contacts
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "name": c.name,
                        "city": c.city,
                        "vat_id": c.vat_id,
                        "email": c.email,
                    })
                })
                .collect();
            emit(ctx, json!({ "contacts": data }), render_contact_list);
            Ok(())
        }
    }
}

fn fmt_line(l: &invoice::InvoiceLine) -> Value {
    json!({
        "line_no": l.line_no,
        "description": l.description,
        "quantity": l.quantity,
        "unit_price_cents": l.unit_price_cents,
        "unit_price": format_amount(l.unit_price_cents),
        "vat_code": l.vat_code,
        "vat_rate_bp": l.vat_rate_bp,
        "amount_cents": l.amount_cents,
        "amount": format_amount(l.amount_cents),
        "vat_amount_cents": l.vat_amount_cents,
        "vat_amount": format_amount(l.vat_amount_cents),
    })
}

fn fmt_invoice(inv: &invoice::Invoice) -> Value {
    json!({
        "id": inv.id,
        "invoice_number": inv.invoice_number,
        "invoice_type": inv.invoice_type,
        "contact_id": inv.contact_id,
        "contact_name": inv.contact.as_ref().map(|c| c.name.clone()),
        "date": inv.date,
        "due_date": inv.due_date,
        "delivery_date": inv.delivery_date,
        "status": inv.status,
        "reference": inv.reference,
        "notes": inv.notes,
        "entry_id": inv.entry_id,
        "credit_for_invoice_id": inv.credit_for_invoice_id,
        "net_cents": inv.net_cents,
        "vat_cents": inv.vat_cents,
        "gross_cents": inv.gross_cents,
        "paid_cents": inv.paid_cents,
        "outstanding_cents": inv.gross_cents - inv.paid_cents,
        "net": format_amount(inv.net_cents),
        "vat": format_amount(inv.vat_cents),
        "gross": format_amount(inv.gross_cents),
        "paid": format_amount(inv.paid_cents),
        "lines": inv.lines.iter().map(fmt_line).collect::<Vec<_>>(),
        "payments": inv
            .payments
            .iter()
            .map(|p| {
                json!({
                    "date": p.date,
                    "amount": format_amount(p.amount_cents),
                    "method": p.method,
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn contact_json(c: &invoice::Contact) -> Value {
    json!({
        "id": c.id,
        "name": c.name,
        "address": c.address,
        "postal_code": c.postal_code,
        "city": c.city,
        "country": c.country,
        "email": c.email,
        "vat_id": c.vat_id,
        "kvk": c.kvk,
        "created_by": c.created_by,
        "created_at": c.created_at,
    })
}

fn cmd_invoice(ctx: &Ctx, cmd: &InvoiceCmd) -> bukio_cli::Result<()> {
    match cmd {
        InvoiceCmd::Create(a) => {
            let conn = ensure_db(ctx)?;
            let lines = invoice::split_line_specs(&[a.lines.clone()]);
            let specs: Vec<invoice::InvoiceLineSpec> = invoice::validate_invoice_lines(&conn, &lines)?
                .into_iter()
                .map(|p| invoice::InvoiceLineSpec {
                    qty: p.qty,
                    description: p.description,
                    price_cents: p.price_cents,
                    vat_code: p.vat_code,
                })
                .collect();
            let contact_id: i64 = a.contact.parse().unwrap_or(0);
            let contact_id = if contact_id == 0 {
                let found = invoice::list_contacts(&conn)?
                    .into_iter()
                    .find(|c| c.name == a.contact)
                    .ok_or_else(|| {
                        AppError::new(
                            "CONTACT_NOT_FOUND",
                            format!("contact '{}' does not exist", a.contact),
                        )
                    })?;
                found.id
            } else {
                contact_id
            };
            let inv = invoice::create_invoice(
                &conn,
                contact_id,
                &specs,
                &a.date,
                Some(a.due_days.parse().unwrap_or(30)),
                a.delivery_date.as_deref(),
                a.description.as_deref(),
                a.reference.as_deref(),
                a.notes.as_deref(),
                &ctx.actor,
            )?;
            emit(
                ctx,
                json!({ "invoice": fmt_invoice(&inv), "dryRun": false }),
                render_invoice,
            );
            Ok(())
        }
        InvoiceCmd::Finalize(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.id.parse().unwrap_or(0);
            let result = invoice::finalize_invoice(&conn, id, &ctx.actor, ctx.dry_run)?;
            if result.dry_run {
                let postings: Vec<Value> = result
                    .postings
                    .unwrap_or_default()
                    .iter()
                    .map(|p| {
                        let mut obj = serde_json::Map::new();
                        obj.insert("code".into(), json!(p.code));
                        obj.insert("amountCents".into(), json!(p.amount_cents));
                        if let Some(vc) = &p.vat_code {
                            obj.insert("vatCode".into(), json!(vc));
                        }
                        if let Some(va) = p.vat_amount_cents {
                            obj.insert("vatAmountCents".into(), json!(va));
                        }
                        obj.into()
                    })
                    .collect();
                emit(
                    ctx,
                    json!({
                        "invoice_number": result.invoice_number,
                        "postings": postings,
                        "net": result.net_cents,
                        "vat": result.vat_cents,
                        "gross": result.gross_cents,
                        "dryRun": true,
                    }),
                    render_finalize_plan,
                );
            } else {
                let inv = result.invoice.as_ref().unwrap();
                emit(
                    ctx,
                    json!({
                        "invoice": fmt_invoice(inv),
                        "entry": { "id": result.entry_id, "state": result.entry_state },
                        "dryRun": false,
                    }),
                    render_invoice,
                );
            }
            Ok(())
        }
        InvoiceCmd::List(a) => {
            let conn = ensure_db(ctx)?;
            let list = invoice::list_invoices(&conn, a.status.as_deref(), a.inv_type.as_deref())?;
            let data: Vec<Value> = list.iter().map(fmt_invoice).collect();
            emit(ctx, json!({ "invoices": data }), render_invoice_list);
            Ok(())
        }
        InvoiceCmd::Show(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.id.parse().unwrap_or(0);
            let inv = invoice::get_invoice(&conn, id)?.ok_or_else(|| {
                AppError::new("NOT_FOUND", format!("invoice {id} does not exist"))
            })?;
            emit(ctx, json!({ "invoice": fmt_invoice(&inv) }), render_invoice);
            Ok(())
        }
        InvoiceCmd::Pdf(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.id.parse().unwrap_or(0);
            let inv = invoice::get_invoice(&conn, id)?.ok_or_else(|| {
                AppError::new("NOT_FOUND", format!("invoice {id} does not exist"))
            })?;
            let out = a
                .out
                .clone()
                .unwrap_or_else(|| format!("invoice-{}.pdf", inv.invoice_number.as_deref().unwrap_or("draft")));
            let (bytes, path) = invoice::invoice_to_pdf(&conn, &inv, &out)?;
            emit(
                ctx,
                json!({ "path": path, "bytes": bytes }),
                |_c, d| println!("wrote {} ({} bytes)", d["path"], d["bytes"]),
            );
            Ok(())
        }
        InvoiceCmd::Ubl(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.id.parse().unwrap_or(0);
            let inv = invoice::get_invoice(&conn, id)?.ok_or_else(|| {
                AppError::new("NOT_FOUND", format!("invoice {id} does not exist"))
            })?;
            let xml = invoice::invoice_to_ubl(&conn, &inv)?;
            if let Some(out) = &a.out {
                std::fs::write(out, &xml)
                    .map_err(|e| AppError::new("IO_ERROR", format!("cannot write {out}: {e}")))?;
                emit(ctx, json!({ "path": out, "bytes": xml.len() }), |_c, d| {
                    println!("wrote {} ({} bytes)", d["path"], d["bytes"])
                });
            } else {
                emit(ctx, json!({ "ubl": xml }), |_c, d| {
                    println!("{}", d["ubl"].as_str().unwrap_or(""))
                });
            }
            Ok(())
        }
        InvoiceCmd::Credit(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.id.parse().unwrap_or(0);
            let credit = invoice::credit_invoice(
                &conn,
                id,
                a.date.as_deref(),
                a.reason.as_deref(),
                &ctx.actor,
            )?;
            emit(
                ctx,
                json!({ "invoice": fmt_invoice(&credit), "dryRun": false }),
                render_invoice,
            );
            Ok(())
        }
        InvoiceCmd::PeppolSend(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.id.parse().unwrap_or(0);
            let inv = invoice::get_invoice(&conn, id)?.ok_or_else(|| {
                AppError::new("NOT_FOUND", format!("invoice {id} does not exist"))
            })?;
            let result = invoice::send_peppol_invoice(&conn, &inv, a.endpoint.as_deref(), ctx.dry_run)?;
            emit(ctx, result, |_c, d| {
                println!(
                    "peppol {}: invoice {} -> {}",
                    if d["dryRun"].as_bool().unwrap_or(false) { "dry-run" } else { "sent" },
                    d["invoice_number"].as_str().unwrap_or(""),
                    d["endpoint"].as_str().unwrap_or(""),
                );
                if let Some(status) = d["status"].as_u64() {
                    println!("provider status {status}");
                }
                if let Some(body) = d["response"].as_str() {
                    println!("{body}");
                }
            });
            Ok(())
        }
        InvoiceCmd::Pay(a) => {
            let conn = ensure_db(ctx)?;
            let id: i64 = a.id.parse().unwrap_or(0);
            let amount_cents = match &a.amount {
                Some(amt) => (amt.parse::<f64>().unwrap_or(0.0) * 100.0).round() as i64,
                None => {
                    let inv = invoice::get_invoice(&conn, id)?.ok_or_else(|| {
                        AppError::new("NOT_FOUND", format!("invoice {id} does not exist"))
                    })?;
                    inv.gross_cents - inv.paid_cents
                }
            };
            let inv = invoice::mark_paid(
                &conn,
                id,
                &a.date,
                amount_cents,
                &a.method,
                None,
                &ctx.actor,
            )?;
            emit(ctx, json!({ "invoice": fmt_invoice(&inv) }), render_invoice);
            Ok(())
        }
    }
}

fn render_contact(_ctx: &Ctx, d: &Value) {
    let c = &d["contact"];
    println!(
        "contact #{}  {}  ({})",
        c["id"], c["name"], c["country"]
    );
    println!(
        "  {}  {}  {}",
        c["address"].as_str().unwrap_or(""),
        c["postal_code"].as_str().unwrap_or(""),
        c["city"].as_str().unwrap_or("")
    );
    if let Some(vat) = c["vat_id"].as_str() {
        println!("  BTW {vat}");
    }
}

fn render_contact_list(_ctx: &Ctx, d: &Value) {
    let rows: Vec<Vec<String>> = d["contacts"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| {
                    vec![
                        c["id"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                        c["name"].as_str().unwrap_or("").to_string(),
                        c["city"].as_str().unwrap_or("").to_string(),
                        c["vat_id"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&rows, &hdr(&["#", "name", "city", "vat"]));
}

fn render_invoice(_ctx: &Ctx, d: &Value) {
    let inv = d["invoice"].as_object().unwrap();
    println!(
        "invoice #{}  {}  [{}]  {}  {}",
        inv["id"].as_i64().unwrap_or(0),
        inv["invoice_number"].as_str().unwrap_or("concept"),
        inv["status"].as_str().unwrap_or(""),
        inv["date"].as_str().unwrap_or(""),
        inv["gross"].as_str().unwrap_or("")
    );
    if let Some(c) = inv["contact"].as_object() {
        println!("  to: {}", c["name"].as_str().unwrap_or(""));
    }
    for l in inv["lines"].as_array().unwrap_or(&Vec::new()) {
        println!(
            "  {}x {:<40} {:>10}",
            l["quantity"].as_i64().unwrap_or(0),
            l["description"].as_str().unwrap_or(""),
            l["amount"].as_str().unwrap_or("")
        );
    }
    println!(
        "  net {}  vat {}  gross {}  paid {}",
        inv["net"].as_str().unwrap_or(""),
        inv["vat"].as_str().unwrap_or(""),
        inv["gross"].as_str().unwrap_or(""),
        inv["paid"].as_str().unwrap_or("")
    );
}

fn render_invoice_list(_ctx: &Ctx, d: &Value) {
    let rows: Vec<Vec<String>> = d["invoices"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|inv| {
                    vec![
                        inv["id"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                        inv["invoice_number"].as_str().unwrap_or("concept").to_string(),
                        inv["invoice_type"].as_str().unwrap_or("").to_string(),
                        inv["status"].as_str().unwrap_or("").to_string(),
                        inv["date"].as_str().unwrap_or("").to_string(),
                        inv["gross"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&rows, &hdr(&["#", "number", "type", "status", "date", "gross"]));
}

fn render_finalize_plan(_ctx: &Ctx, d: &Value) {
    println!(
        "plan: finalize invoice as {} (net {} vat {} gross {})",
        d["invoice_number"].as_str().unwrap_or(""),
        d["net"].as_str().unwrap_or(""),
        d["vat"].as_str().unwrap_or(""),
        d["gross"].as_str().unwrap_or("")
    );
    for p in d["postings"].as_array().unwrap_or(&Vec::new()) {
        println!("  {}  {}", p["code"].as_str().unwrap_or(""), p["amount"].as_str().unwrap_or(""));
    }
    println!("(dry run — nothing written)");
}

#[derive(Subcommand)]
enum ContactCmd {
    /// add a contact
    Add(ContactAddArgs),
    /// list contacts
    List,
}

#[derive(Args)]
struct ContactAddArgs {
    #[arg(long)]
    name: String,
    #[arg(long)]
    address: Option<String>,
    #[arg(long)]
    postal_code: Option<String>,
    #[arg(long)]
    city: Option<String>,
    #[arg(long, default_value = "NL")]
    country: String,
    #[arg(long)]
    email: Option<String>,
    #[arg(long)]
    vat_id: Option<String>,
    #[arg(long)]
    kvk: Option<String>,
}

#[derive(Subcommand)]
enum InvoiceCmd {
    /// create a draft invoice (compliance-validated at finalize)
    Create(InvoiceCreateArgs),
    /// assign the sequential number and book the entry
    Finalize(InvoiceFinalizeArgs),
    /// list invoices
    List(InvoiceListArgs),
    /// show one invoice with lines and payments
    Show(InvoiceShowArgs),
    /// render the invoice to PDF (headless Chromium)
    Pdf(InvoicePdfArgs),
    /// export UBL 2.1 / Peppol BIS 3.0 XML
    Ubl(InvoiceUblArgs),
    /// create a credit note (draft) for a finalized sales invoice
    Credit(InvoiceCreditArgs),
    /// send the invoice to a Peppol access-point provider
    PeppolSend(InvoicePeppolArgs),
    /// record a payment (tracking; the posting comes from the bank flow)
    Pay(InvoicePayArgs),
}

#[derive(Args)]
struct InvoiceCreateArgs {
    #[arg(long)]
    contact: String,
    #[arg(long)]
    lines: String,
    #[arg(long)]
    date: String,
    #[arg(long, default_value = "30")]
    due_days: String,
    #[arg(long)]
    delivery_date: Option<String>,
    #[arg(long)]
    description: Option<String>,
    #[arg(long)]
    reference: Option<String>,
    #[arg(long)]
    notes: Option<String>,
}

#[derive(Args)]
struct InvoiceFinalizeArgs {
    #[arg(long)]
    id: String,
}

#[derive(Args)]
struct InvoiceListArgs {
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    inv_type: Option<String>,
}

#[derive(Args)]
struct InvoiceShowArgs {
    #[arg(long)]
    id: String,
}

#[derive(Args)]
struct InvoicePdfArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    out: Option<String>,
}

#[derive(Args)]
struct InvoiceUblArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    out: Option<String>,
}

#[derive(Args)]
struct InvoiceCreditArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    date: Option<String>,
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Args)]
struct InvoicePeppolArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    endpoint: Option<String>,
}

#[derive(Args)]
struct InvoicePayArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    date: String,
    #[arg(long)]
    amount: Option<String>,
    #[arg(long, default_value = "bank")]
    method: String,
}

// --- report ---------------------------------------------------------------

fn current_year() -> String {
    chrono::Utc::now().format("%Y").to_string()
}

/// Shared report emit: json | csv | xlsx | human (mirrors Node emitReport).
fn emit_report(
    ctx: &Ctx,
    fmt: &ReportFmtArgs,
    data: Value,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    sheet_name: &str,
    render: fn(&Ctx, &Value),
) -> bukio_cli::Result<()> {
    let format = fmt
        .format
        .clone()
        .unwrap_or_else(|| if ctx.json { "json".to_string() } else { "human".to_string() });
    match format.as_str() {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&json!({ "ok": true, "data": data })).unwrap());
        }
        "csv" => {
            let csv = report::export::to_csv(&rows, &headers);
            match &fmt.out {
                Some(out) => {
                    std::fs::write(out, csv)?;
                    println!("wrote {out}");
                }
                None => {
                    print!("{csv}");
                }
            }
        }
        "xlsx" => {
            let out = fmt
                .out
                .clone()
                .ok_or_else(|| AppError::new("OUT_REQUIRED", "--out <path> is required for xlsx output"))?;
            if let Some(parent) = Path::new(&out).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            let sheet = report::export::SheetSpec { name: sheet_name.to_string(), headers, rows };
            report::export::write_xlsx(&out, &[sheet])?;
            println!("wrote {out}");
        }
        _ => render(ctx, &data),
    }
    Ok(())
}

fn hdr(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn cmd_report(ctx: &Ctx, cmd: &ReportCmd) -> bukio_cli::Result<()> {
    match cmd {
        ReportCmd::TrialBalance(a) => {
            let conn = ensure_db(ctx)?;
            let tb = report::trial_balance::trial_balance(&conn, a.year.as_deref())?;
            let accounts: Vec<Value> = tb
                .accounts
                .iter()
                .map(|acc| {
                    json!({
                        "code": acc.code, "name": acc.name, "type": acc.account_type,
                        "debit_cents": acc.debit_cents, "credit_cents": acc.credit_cents, "net_cents": acc.net_cents,
                        "debit": format_amount(acc.debit_cents), "credit": format_amount(acc.credit_cents), "net": format_amount(acc.net_cents),
                    })
                })
                .collect();
            let data = json!({
                "year": a.year,
                "accounts": accounts,
                "total_debit_cents": tb.total_debit_cents,
                "total_credit_cents": tb.total_credit_cents,
                "total_debit": format_amount(tb.total_debit_cents),
                "total_credit": format_amount(tb.total_credit_cents),
                "balanced": tb.balanced,
            });
            let mut rows: Vec<Vec<String>> = tb
                .accounts
                .iter()
                .map(|acc| {
                    vec![
                        acc.code.clone(),
                        acc.name.clone(),
                        acc.account_type.clone(),
                        format_amount(acc.debit_cents),
                        format_amount(acc.credit_cents),
                        format_amount(acc.net_cents),
                    ]
                })
                .collect();
            rows.push(vec![
                String::new(),
                "TOTAAL".to_string(),
                String::new(),
                format_amount(tb.total_debit_cents),
                format_amount(tb.total_credit_cents),
                format_amount(tb.total_debit_cents),
            ]);
            emit_report(ctx, &a.fmt, data, hdr(&["code", "account", "type", "debit", "credit", "net"]), rows, "Trial balance", render_tb)?;
            Ok(())
        }
        ReportCmd::Balans(a) => {
            let conn = ensure_db(ctx)?;
            let as_of = a.as_of.clone().unwrap_or_else(today_iso);
            let b = report::balans::balans(&conn, &as_of)?;
            let data = json!({
                "as_of": b.as_of,
                "assets": {
                    "total_cents": b.assets.total_cents,
                    "total": format_amount(b.assets.total_cents),
                    "sections": serde_json::to_value(&b.assets.sections).unwrap(),
                },
                "liabilities_and_equity": {
                    "total_cents": b.liabilities_and_equity.total_cents,
                    "total": format_amount(b.liabilities_and_equity.total_cents),
                    "sections": serde_json::to_value(&b.liabilities_and_equity.sections).unwrap(),
                    "result_cents": b.liabilities_and_equity.result_cents,
                    "result": format_amount(b.liabilities_and_equity.result_cents),
                },
                "balanced": b.balanced,
            });
            let mut rows: Vec<Vec<String>> = Vec::new();
            for s in &b.assets.sections {
                for acc in &s.accounts {
                    rows.push(vec![
                        "activa".to_string(),
                        s.rgs_code.clone().unwrap_or_default(),
                        s.label.clone(),
                        acc.code.clone(),
                        acc.name.clone(),
                        format_amount(acc.balance_cents),
                    ]);
                }
            }
            for s in &b.liabilities_and_equity.sections {
                for acc in &s.accounts {
                    rows.push(vec![
                        "passiva".to_string(),
                        s.rgs_code.clone().unwrap_or_default(),
                        s.label.clone(),
                        acc.code.clone(),
                        acc.name.clone(),
                        format_amount(acc.balance_cents),
                    ]);
                }
            }
            rows.push(vec![
                "passiva".to_string(),
                String::new(),
                "Nog te verdelen resultaat".to_string(),
                String::new(),
                String::new(),
                format_amount(b.liabilities_and_equity.result_cents),
            ]);
            emit_report(ctx, &a.fmt, data, hdr(&["side", "rgs", "group", "code", "name", "amount"]), rows, "Balans", render_balans)?;
            Ok(())
        }
        ReportCmd::Pnl(a) => {
            let conn = ensure_db(ctx)?;
            let year = a.year.clone().unwrap_or_else(current_year);
            let from = a.from.clone().unwrap_or_else(|| format!("{year}-01-01"));
            let to = a.to.clone().unwrap_or_else(|| format!("{year}-12-31"));
            let p = report::pnl::pnl(&conn, &from, &to)?;
            let data = json!({
                "from": p.from, "to": p.to,
                "sections": serde_json::to_value(&p.sections).unwrap(),
                "revenue_cents": p.revenue_cents, "revenue": format_amount(p.revenue_cents),
                "costs_cents": p.costs_cents, "costs": format_amount(p.costs_cents),
                "result_cents": p.result_cents, "result": format_amount(p.result_cents),
            });
            let mut rows: Vec<Vec<String>> = Vec::new();
            for s in &p.sections {
                for acc in &s.accounts {
                    rows.push(vec![
                        s.rgs_code.clone().unwrap_or_default(),
                        s.label.clone(),
                        acc.code.clone(),
                        acc.name.clone(),
                        format_amount(acc.amount_cents),
                    ]);
                }
            }
            rows.push(vec![
                String::new(),
                "Netto resultaat".to_string(),
                String::new(),
                String::new(),
                format_amount(p.result_cents),
            ]);
            emit_report(ctx, &a.fmt, data, hdr(&["rgs", "group", "code", "name", "amount"]), rows, "Winst en verlies", render_pnl)?;
            Ok(())
        }
        ReportCmd::Journal(a) => {
            let conn = ensure_db(ctx)?;
            let year = a.year.clone().unwrap_or_else(current_year);
            let from = a.from.clone().unwrap_or_else(|| format!("{year}-01-01"));
            let to = a.to.clone().unwrap_or_else(|| format!("{year}-12-31"));
            let rows = report::journal::journal(&conn, &from, &to)?;
            let data = json!({
                "from": from, "to": to,
                "rows": rows.iter().map(|r| json!({
                    "entry_id": r.entry_id, "date": r.date, "description": r.description,
                    "source": r.source, "state": r.state, "created_by": r.created_by,
                    "amount_cents": r.amount_cents,
                    "account_code": r.account_code, "account_name": r.account_name, "account_type": r.account_type,
                    "amount": r.amount_cents.map(format_amount).unwrap_or_default(),
                })).collect::<Vec<_>>(),
            });
            let csv_rows: Vec<Vec<String>> = rows
                .iter()
                .map(|r| {
                    vec![
                        r.date.clone(),
                        r.entry_id.to_string(),
                        r.description.clone(),
                        r.source.clone(),
                        r.state.clone(),
                        r.account_code.clone().unwrap_or_default(),
                        r.account_name.clone().unwrap_or_default(),
                        r.amount_cents.map(format_amount).unwrap_or_default(),
                    ]
                })
                .collect();
            emit_report(ctx, &a.fmt, data, hdr(&["date", "entry", "description", "source", "state", "code", "account", "amount"]), csv_rows, "Journal", render_journal)?;
            Ok(())
        }
    }
}

/// Simple aligned text table for human output (mirrors util.table).
fn table(rows: &[Vec<String>], headers: &[String]) {
    let widths: Vec<usize> = (0..headers.len())
        .map(|i| {
            headers[i].chars().count().max(
                rows.iter()
                    .map(|r| r.get(i).map(|c| c.chars().count()).unwrap_or(0))
                    .max()
                    .unwrap_or(0),
            )
        })
        .collect();
    let line = |cells: &[String]| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}{}", c, " ".repeat(widths[i].saturating_sub(c.chars().count()))))
            .collect::<Vec<_>>()
            .join("  ")
    };
    println!("{}", line(headers));
    println!("{}", widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("  "));
    for r in rows {
        println!("{}", line(r));
    }
}

fn render_tb(_ctx: &Ctx, d: &Value) {
    let accounts: Vec<Value> = d["accounts"].as_array().cloned().unwrap_or_default();
    let rows: Vec<Vec<String>> = accounts
        .iter()
        .map(|a| {
            vec![
                a["code"].as_str().unwrap_or("").to_string(),
                a["name"].as_str().unwrap_or("").to_string(),
                a["type"].as_str().unwrap_or("").to_string(),
                a["debit"].as_str().unwrap_or("").to_string(),
                a["credit"].as_str().unwrap_or("").to_string(),
                a["net"].as_str().unwrap_or("").to_string(),
            ]
        })
        .collect();
    table(&rows, &hdr(&["code", "account", "type", "debit", "credit", "net"]));
    println!(
        "totals:  debit {}  credit {}  -> {}",
        d["total_debit"].as_str().unwrap_or(""),
        d["total_credit"].as_str().unwrap_or(""),
        if d["balanced"].as_bool().unwrap_or(false) { "BALANCED" } else { "UNBALANCED!" }
    );
}

fn render_balans(_ctx: &Ctx, d: &Value) {
    let fmt = |c: &Value| c.as_i64().map(format_amount).unwrap_or_default();
    println!("BALANS as of {}", d["as_of"].as_str().unwrap_or(""));
    println!("ACTIVA");
    for s in d["assets"]["sections"].as_array().unwrap_or(&Vec::new()) {
        println!("  {} ({})", s["label"].as_str().unwrap_or(""), s["rgs_code"].as_str().unwrap_or("null"));
        for a in s["accounts"].as_array().unwrap_or(&Vec::new()) {
            println!("    {}  {:<30} {}", a["code"].as_str().unwrap_or(""), a["name"].as_str().unwrap_or(""), fmt(&a["balance_cents"]));
        }
        println!("    {:<32} {}", "", fmt(&s["total_cents"]));
    }
    println!("  totaal activa: {}", d["assets"]["total"].as_str().unwrap_or(""));
    println!("PASSIVA");
    for s in d["liabilities_and_equity"]["sections"].as_array().unwrap_or(&Vec::new()) {
        println!("  {} ({})", s["label"].as_str().unwrap_or(""), s["rgs_code"].as_str().unwrap_or("null"));
        for a in s["accounts"].as_array().unwrap_or(&Vec::new()) {
            println!("    {}  {:<30} {}", a["code"].as_str().unwrap_or(""), a["name"].as_str().unwrap_or(""), fmt(&a["balance_cents"]));
        }
        println!("    {:<32} {}", "", fmt(&s["total_cents"]));
    }
    println!("  Nog te verdelen resultaat  {}", d["liabilities_and_equity"]["result"].as_str().unwrap_or(""));
    println!("  totaal passiva: {}", d["liabilities_and_equity"]["total"].as_str().unwrap_or(""));
    println!("{}", if d["balanced"].as_bool().unwrap_or(false) { "BALANCED" } else { "UNBALANCED!" });
}

fn render_pnl(_ctx: &Ctx, d: &Value) {
    let fmt = |c: &Value| c.as_i64().map(format_amount).unwrap_or_default();
    println!("WINST-EN-VERLIESREKENING {} .. {}", d["from"].as_str().unwrap_or(""), d["to"].as_str().unwrap_or(""));
    for s in d["sections"].as_array().unwrap_or(&Vec::new()) {
        println!("  {} ({})", s["label"].as_str().unwrap_or(""), s["rgs_code"].as_str().unwrap_or("null"));
        for a in s["accounts"].as_array().unwrap_or(&Vec::new()) {
            println!("    {}  {:<30} {}", a["code"].as_str().unwrap_or(""), a["name"].as_str().unwrap_or(""), fmt(&a["amount_cents"]));
        }
        println!("    {:<32} {}", "", fmt(&s["total_cents"]));
    }
    println!("  opbrengsten: {}", d["revenue"].as_str().unwrap_or(""));
    println!("  kosten:      {}", d["costs"].as_str().unwrap_or(""));
    println!("  resultaat:   {}", d["result"].as_str().unwrap_or(""));
}

fn render_journal(_ctx: &Ctx, d: &Value) {
    let rows: Vec<Vec<String>> = d["rows"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|r| {
                    vec![
                        r["date"].as_str().unwrap_or("").to_string(),
                        r["entry_id"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                        r["state"].as_str().unwrap_or("").to_string(),
                        r["account_code"].as_str().unwrap_or("").to_string(),
                        r["account_name"].as_str().unwrap_or("").to_string(),
                        r["amount"].as_str().unwrap_or("").to_string(),
                        r["description"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&rows, &hdr(&["date", "#", "state", "code", "account", "amount", "description"]));
}

// --- human renderers (mirror the Node console output) ---------------------

fn render_init(_ctx: &Ctx, d: &Value) {
    let c = &d["company"];
    println!("company:  {} ({})", c["name"].as_str().unwrap_or(""), c["legal_form"].as_str().unwrap_or(""));
    println!("kvk:      {}", c["kvk"].as_str().unwrap_or("-"));
    println!("btw-id:   {}", c["btw_id"].as_str().unwrap_or("-"));
    let vat_on = c["vat_module"].as_i64().unwrap_or(0) == 1;
    let kor = c["kor_flag"].as_i64().unwrap_or(0) == 1;
    println!("vat:      {}{}", if vat_on { "on" } else { "off" }, if kor { " (KOR)" } else { "" });
    println!("db:       {}", d["db"].as_str().unwrap_or(""));
    println!("chart:    {} accounts (default chart)", d["chart"]["accounts"].as_i64().unwrap_or(0));
    if let Some(created) = d["chart"]["created"].as_i64() {
        println!("seeded:   {created} new");
    }
    if vat_on {
        println!("vat:      module enabled (incl. 1500/2500)");
    }
    println!("{}", if d["dryRun"].as_bool().unwrap_or(false) { "(dry run — nothing written)" } else { "initialised." });
}

fn render_entry(_ctx: &Ctx, e: &Value) {
    println!("entry #{}  [{}]  {}  {}", e["id"], e["state"], e["date"], e["description"]);
    let postings = e["postings"].as_array().cloned().unwrap_or_default();
    let mut name_width = 28usize;
    for p in &postings {
        let n = p["account_name"].as_str().unwrap_or("").chars().count();
        if n > name_width {
            name_width = n;
        }
    }
    for p in &postings {
        let name = p["account_name"].as_str().unwrap_or("");
        println!("  {}  {}{} {}", p["account_code"], name, " ".repeat(name_width.saturating_sub(name.chars().count())), p["amount"]);
    }
    let total: i64 = postings.iter().map(|p| p["amount_cents"].as_i64().unwrap_or(0)).sum();
    println!("  {}{} (sum)", " ".repeat(name_width + 3), format_amount(total));
}

fn render_entry_plan(_ctx: &Ctx, d: &Value) {
    println!("plan: create entry {} \"{}\"", d["date"], d["description"]);
    for p in d["postings"].as_array().unwrap_or(&Vec::new()) {
        let fx = p["fx_currency"].as_str().map(|c| format!("  [{c} ...]")).unwrap_or_default();
        println!("  {}  {}{}", p["code"], p["amount"], fx);
    }
    println!("  sum: {}{}", d["sum"], if d["post"].as_bool().unwrap_or(false) { "  -> will post" } else { "" });
    println!("(dry run — nothing written)");
}

fn render_post_plan(_ctx: &Ctx, d: &Value) {
    println!("plan: post entry #{} ({} -> {})", d["id"], d["current_state"], d["target_state"]);
    println!("(dry run — nothing written)");
}

fn render_reverse_plan(_ctx: &Ctx, d: &Value) {
    println!("plan: reverse entry #{} (state: {})", d["id"], d["current_state"]);
    for p in d["reversed_postings"].as_array().unwrap_or(&Vec::new()) {
        println!("  {}  {}", p["account_code"], p["amount"]);
    }
    println!("(dry run — nothing written)");
}

fn render_entry_list(_ctx: &Ctx, d: &Value) {
    println!("{:<4} {:<11} {:<8} {:<10} {:<14} {}", "#", "date", "state", "source", "by", "description");
    println!("{}", "-".repeat(70));
    for e in d["entries"].as_array().unwrap_or(&Vec::new()) {
        println!("{:<4} {:<11} {:<8} {:<10} {:<14} {}", e["id"], e["date"], e["state"], e["source"], e["created_by"], e["description"]);
    }
}

fn render_account(_ctx: &Ctx, a: &Value) {
    println!("{}  {}  [{}]  {}", a["code"], a["name"], a["type"], a["normal_balance"]);
    if let Some(rgs) = a["rgs_code"].as_str() {
        println!("    rgs: {rgs}");
    }
}

fn render_account_list(_ctx: &Ctx, d: &Value) {
    println!("{:<8} {:<30} {:<10} {:<8}", "code", "name", "type", "balance");
    println!("{}", "-".repeat(60));
    for a in d["accounts"].as_array().unwrap_or(&Vec::new()) {
        println!("{:<8} {:<30} {:<10} {:<8}", a["code"], a["name"], a["type"], a["normal_balance"]);
    }
}

fn render_import(_ctx: &Ctx, d: &Value) {
    println!("imported {} accounts ({} skipped of {})", d["created"], d["skipped"], d["total"]);
    for e in d["errors"].as_array().unwrap_or(&Vec::new()) {
        println!("  line {}: {}", e["line"], e["error"]);
    }
}

fn render_vat_enable(_ctx: &Ctx, d: &Value) {
    println!("VAT module enabled: accounts {} codes {}", d["accounts"], d["codes"]);
}

fn render_vat_enable_plan(_ctx: &Ctx, d: &Value) {
    println!("plan: enable VAT module");
    let accounts: Vec<String> = d["accounts"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    let codes: Vec<String> = d["codes"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    println!("  accounts: {}", accounts.join(", "));
    println!("  codes:    {}", codes.join(", "));
    println!("(dry run — nothing written)");
}

fn render_vat_book_plan(_ctx: &Ctx, d: &Value) {
    let currency = d["currency"].as_str().unwrap_or("");
    let suffix = if currency.is_empty() { String::new() } else { format!(" ({currency} -> EUR)") };
    println!(
        "plan: VAT entry {} \"{}\"{}",
        d["date"].as_str().unwrap_or(""),
        d["description"].as_str().unwrap_or(""),
        suffix
    );
    for p in d["postings"].as_array().unwrap_or(&Vec::new()) {
        let fx = p["fx_currency"].as_str().map(|c| format!("  [{c} ...]")).unwrap_or_default();
        let vat = p["vat_code"].as_str().map(|c| format!("  @{c} ({})", p["vat_amount"].as_str().unwrap_or(""))).unwrap_or_default();
        println!("  {}  {}{}{}", p["code"].as_str().unwrap_or(""), p["amount"].as_str().unwrap_or(""), fx, vat);
    }
    println!("(dry run — nothing written)");
}

fn render_vat_book(_ctx: &Ctx, d: &Value) {
    let e = &d["entry"];
    println!(
        "entry #{}  [{}]  {}  {}",
        e["id"], e["state"], e["date"], e["description"]
    );
    for p in e["postings"].as_array().unwrap_or(&Vec::new()) {
        let name = p["account_name"].as_str().unwrap_or("");
        let vat = p["vat_amount"].as_str().filter(|s| !s.is_empty()).map(|s| format!("  vat {s}")).unwrap_or_default();
        println!(
            "  {}  {}{} {}{}",
            p["account_code"].as_str().unwrap_or(""),
            name,
            " ".repeat(28usize.saturating_sub(name.chars().count())),
            p["amount"].as_str().unwrap_or(""),
            vat
        );
    }
}

fn render_readout(_ctx: &Ctx, d: &Value) {
    let fmt = |k: &str| d["fields"][k]["amount"].as_str().unwrap_or("").to_string();
    println!(
        "OB-AANGIFTE {} ({} .. {}) — manual filing aid",
        d["period"].as_str().unwrap_or(""),
        d["from"].as_str().unwrap_or(""),
        d["to"].as_str().unwrap_or("")
    );
    println!("  1a  omzet hoog           {}", fmt("1a"));
    println!("  1b  omzet laag           {}", fmt("1b"));
    println!("  1c  omzet 0/vrijgesteld  {}", fmt("1c"));
    println!("  1d  privégebruik         {}", fmt("1d"));
    println!("  3a  inkopen hoog         {}", fmt("3a"));
    println!("  3b  inkopen laag         {}", fmt("3b"));
    println!("  3c  inkopen 0/verlegd    {}", fmt("3c"));
    println!("  4a  verlegd binnenland   {}", fmt("4a"));
    println!("  4b  verlegd EU           {}", fmt("4b"));
    println!("  5a  verschuldigde btw    {}", fmt("5a"));
    println!("  5b  voorbelasting        {}", fmt("5b"));
    println!("  5d  te betalen/ontvangen {}", d["to_pay"].as_str().unwrap_or(""));
    println!("  -> enter these amounts in Mijn Belastingdienst Zakelijk");
}

fn render_mark_filed(_ctx: &Ctx, d: &Value) {
    println!("marked {} as filed", d["period"].as_str().unwrap_or(""));
}

fn render_fx_set(_ctx: &Ctx, d: &Value) {
    println!(
        "{} {}: {} ({})",
        d["currency"].as_str().unwrap_or(""),
        d["date"].as_str().unwrap_or(""),
        d["rate"].as_str().unwrap_or(""),
        d["source"].as_str().unwrap_or("")
    );
}

fn render_fx_list(_ctx: &Ctx, d: &Value) {
    let rows: Vec<Vec<String>> = d["rates"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|r| {
                    vec![
                        r["currency"].as_str().unwrap_or("").to_string(),
                        r["date"].as_str().unwrap_or("").to_string(),
                        r["rate"].as_str().unwrap_or("").to_string(),
                        r["source"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&rows, &hdr(&["currency", "date", "rate", "source"]));
}

fn render_vat_codes(_ctx: &Ctx, d: &Value) {
    for c in d["codes"].as_array().unwrap_or(&Vec::new()) {
        println!("{:<4} {}bp  {}", c["code"], c["rate_bp"], c["description"].as_str().unwrap_or(""));
    }
}

fn render_bank_add(_ctx: &Ctx, d: &Value) {
    let b = &d["bank_account"];
    println!(
        "bank account {} ({}) -> ledger {}",
        b["iban"].as_str().unwrap_or(""),
        b["name"].as_str().unwrap_or("-"),
        b["account_code"].as_str().unwrap_or("")
    );
}

fn render_bank_list(_ctx: &Ctx, d: &Value) {
    let rows: Vec<Vec<String>> = d["accounts"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|a| {
                    vec![
                        a["iban"].as_str().unwrap_or("").to_string(),
                        a["name"].as_str().unwrap_or("").to_string(),
                        a["account_code"].as_str().unwrap_or("").to_string(),
                        a["transaction_count"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                        a["unmatched_count"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                        a["balance"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&rows, &hdr(&["iban", "name", "ledger", "tx", "unmatched", "balance"]));
}

fn render_bank_import_plan(_ctx: &Ctx, d: &Value) {
    println!(
        "plan: import {} transactions to {} — {} new, {} duplicate",
        d["total"], d["iban"], d["imported"], d["duplicates"]
    );
    for t in d["transactions"].as_array().unwrap_or(&Vec::new()) {
        println!(
            "  {}  {:>12}  {}  {}",
            t["date"].as_str().unwrap_or(""),
            t["amount"].as_str().unwrap_or(""),
            t["counterparty"].as_str().unwrap_or(""),
            t["description"].as_str().unwrap_or("")
        );
    }
    println!("(dry run — nothing written)");
}

fn render_bank_import(_ctx: &Ctx, d: &Value) {
    println!(
        "imported {} of {} transactions to {} ({} duplicates skipped)",
        d["imported"], d["total"], d["iban"], d["duplicates"]
    );
}

fn render_bank_tx_list(_ctx: &Ctx, d: &Value) {
    let rows: Vec<Vec<String>> = d["transactions"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|t| {
                    vec![
                        t["id"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                        t["date"].as_str().unwrap_or("").to_string(),
                        t["amount"].as_str().unwrap_or("").to_string(),
                        t["state"].as_str().unwrap_or("").to_string(),
                        t["counterparty"].as_str().unwrap_or("").to_string(),
                        t["description"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&rows, &hdr(&["#", "date", "amount", "state", "counterparty", "description"]));
}

fn render_bank_match_auto(_ctx: &Ctx, d: &Value) {
    let dry = d["dryRun"].as_bool().unwrap_or(false);
    println!(
        "auto-match: {} matched, {} unmatched remaining{}",
        d["matched"].as_array().map(|a| a.len()).unwrap_or(0),
        d["unmatched_remaining"],
        if dry { " (dry run)" } else { "" }
    );
    for m in d["matched"].as_array().unwrap_or(&Vec::new()) {
        if m["kind"].as_str() == Some("invoice") {
            println!(
                "  tx #{} {} {:>12} -> invoice {} ({})",
                m["tx_id"], m["tx_date"], m["amount"], m["invoice_number"], m["contact_name"]
            );
        } else {
            println!(
                "  tx #{} {} {:>12} -> entry #{} ({}, {}d)",
                m["tx_id"], m["tx_date"], m["amount"], m["entry_id"], m["method"], m["day_diff"]
            );
        }
    }
}

fn render_bank_suggest(_ctx: &Ctx, d: &Value) {
    let rows: Vec<Vec<String>> = d["suggestions"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|t| {
                    vec![
                        t["id"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                        t["date"].as_str().unwrap_or("").to_string(),
                        t["amount"].as_str().unwrap_or("").to_string(),
                        t["counterparty"].as_str().unwrap_or("").to_string(),
                        t["suggested_account"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&rows, &hdr(&["#", "date", "amount", "counterparty", "suggest"]));
}

fn render_bank_link(_ctx: &Ctx, d: &Value) {
    println!("linked tx #{} -> entry #{}", d["transaction"]["id"], d["entry_id"]);
}

fn render_bank_post_plan(_ctx: &Ctx, d: &Value) {
    println!("plan: post entry from tx #{} ({})", d["tx"]["id"], d["tx"]["date"]);
    for p in d["postings"].as_array().unwrap_or(&Vec::new()) {
        println!("  {}  {}", p["code"], p["amount"]);
    }
    println!("(dry run — nothing written)");
}

fn render_bank_post(_ctx: &Ctx, d: &Value) {
    println!("posted entry #{} from tx ({} state)", d["entry_id"], d["state"]);
}

fn render_recurring_add_plan(_ctx: &Ctx, d: &Value) {
    println!(
        "plan: {} template \"{}\" ({}, day {}) from {}",
        d["kind"], d["name"], d["frequency"], d["day_of_period"], d["start_date"]
    );
    if d["kind"] == "invoice" {
        println!("  contact #{}: {}", d["contact"], d["lines"]);
    } else {
        println!("  postings: {}", d["postings"]);
    }
    println!("(dry run — nothing written)");
}

fn render_recurring_add(_ctx: &Ctx, d: &Value) {
    let t = &d["template"];
    println!(
        "template #{} \"{}\" [{}] — next run {} ({})",
        t["id"], t["name"], t["kind"], t["next_run_date"], t["frequency"]
    );
    if t["kind"] == "invoice" {
        let lines = t["invoice_lines"].as_array().cloned().unwrap_or_default();
        let parts: Vec<String> = lines
            .iter()
            .map(|l| format!("{}x {}", l["quantity"], l["description"]))
            .collect();
        println!("  contact #{}: {}", t["contact_id"], parts.join(" + "));
    }
}

fn render_recurring_list(_ctx: &Ctx, d: &Value) {
    let rows: Vec<Vec<String>> = d["templates"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|t| {
                    vec![
                        t["id"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                        t["name"].as_str().unwrap_or("").to_string(),
                        t["kind"].as_str().unwrap_or("").to_string(),
                        t["frequency"].as_str().unwrap_or("").to_string(),
                        t["next_run_date"].as_str().unwrap_or("").to_string(),
                        t["runs_done"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                        t["status"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&rows, &hdr(&["#", "name", "kind", "freq", "next", "done", "status"]));
}

fn render_recurring_show(_ctx: &Ctx, d: &Value) {
    let t = &d["template"];
    println!(
        "#{} {} [{}] — {}, day {}",
        t["id"], t["name"], t["status"], t["frequency"], t["day_of_period"]
    );
    println!(
        "  start {}  next {}  runs {}/{}",
        t["start_date"], t["next_run_date"], t["runs_done"], t["runs"]
    );
    for p in t["postings"].as_array().cloned().unwrap_or_default() {
        let vat = p["vat_code"].as_str().map(|c| format!(" @{c}")).unwrap_or_default();
        println!("  {}  {:>12}{}", p["code"], p["amount"], vat);
    }
    if t["final_postings"].is_array() {
        println!("  (final run:)");
        for p in t["final_postings"].as_array().cloned().unwrap_or_default() {
            println!("  {}  {:>12}", p["code"], p["amount"]);
        }
    }
}

fn render_recurring_preview(_ctx: &Ctx, d: &Value) {
    println!("due as of {}: {} template(s) due", d["as_of"], d["templates"].as_array().map(|a| a.len()).unwrap_or(0));
    for t in d["templates"].as_array().cloned().unwrap_or_default() {
        for run in t["runs"].as_array().cloned().unwrap_or_default() {
            if run["kind"] == "invoice" {
                let lines = run["invoice"]["lines"].as_array().cloned().unwrap_or_default();
                let parts: Vec<String> = lines
                    .iter()
                    .map(|l| format!("{}x {}", l["quantity"], l["description"]))
                    .collect();
                println!(
                    "  [INVOICE] {}  {} — {}",
                    run["invoice"]["date"],
                    run["invoice"]["contact_name"].as_str().unwrap_or("contact"),
                    parts.join(" + ")
                );
                continue;
            }
            let kind = if run["kind"] == "reversal" { "REVERSE" } else { "BOOK" };
            println!("  [{kind}] {}  {}", run["entry"]["date"], run["entry"]["description"]);
            for p in run["entry"]["postings"].as_array().cloned().unwrap_or_default() {
                println!("      {}  {:>12}", p["code"], p["amountCents"]);
            }
        }
    }
}

fn render_recurring_run(_ctx: &Ctx, d: &Value) {
    let templates = d["templates"].as_array().cloned().unwrap_or_default();
    let total: usize = templates.iter().map(|t| t["runs"].as_array().map(|a| a.len()).unwrap_or(0)).sum();
    let failed = templates.iter().filter(|t| !t["ok"].as_bool().unwrap_or(false)).count();
    println!(
        "recurring run: {} period(s) across {} template(s){}",
        total,
        templates.len(),
        if d["dry_run"].as_bool().unwrap_or(false) { " (dry run)" } else { "" }
    );
    for t in &templates {
        if !t["ok"].as_bool().unwrap_or(false) {
            println!("  ✗ {}: {} — {}", t["name"], t["error"]["code"], t["error"]["message"]);
            continue;
        }
        for run in t["runs"].as_array().cloned().unwrap_or_default() {
            for e in run["entries"].as_array().cloned().unwrap_or_default() {
                if e["kind"] == "invoice" {
                    println!(
                        "  {}  → draft invoice #{} (finalize to book & number)",
                        e["date"], e["invoice_id"]
                    );
                } else {
                    let action = if e["kind"] == "reversal" { "→ reversal of" } else { "→ booked" };
                    println!("  {}  {} entry #{} ({})", e["date"], action, e["entry_id"], e["state"]);
                }
            }
        }
    }
    if failed > 0 {
        println!("  {failed} template(s) failed — their schedules were left untouched");
    }
}

fn render_depreciation_add_plan(_ctx: &Ctx, d: &Value) {
    println!(
        "plan: depreciate \"{}\" {}/mo for {} months (final {})",
        d["name"], d["monthly"], d["life_months"], d["final"]
    );
    println!(
        "  {} (asset) -{}  /  {} (expense) +{}",
        d["asset"], d["monthly"], d["expense"], d["monthly"]
    );
    println!("(dry run — nothing written)");
}

fn render_depreciation_add(_ctx: &Ctx, d: &Value) {
    println!(
        "template #{} — {}: {}/mo × {} + final {} = {}",
        d["template"]["id"], d["template"]["name"], d["monthly"],
        d["template"]["runs"].as_i64().map(|r| r - 1).unwrap_or(0), d["final"], d["total"]
    );
}

fn render_bank_ignore(_ctx: &Ctx, d: &Value) {
    println!("ignored tx #{}", d["transaction"]["id"]);
}

fn render_bank_unignore(_ctx: &Ctx, d: &Value) {
    println!("re-opened tx #{}", d["transaction"]["id"]);
}
