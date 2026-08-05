//! bukio-cli — CLI entrypoint (clap).
//!
//! Mirrors the Node commander surface. Output contract:
//!   success -> { "ok": true, "data": ... }   (stdout)
//!   failure -> { "ok": false, "error": { "code", "message" } }  (stdout, exit 1)
//! With --json only the JSON document is printed; otherwise human text.

use bukio_cli::core::accounts;
use bukio_cli::core::chart::DEFAULT_CHART;
use bukio_cli::core::db;
use bukio_cli::core::entries::{self, parse_posting_specs};
use bukio_cli::core::money::format_amount;
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
    ListCodes,
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
            let res = vat::enable_vat_module(&conn, &ctx.actor)?;
            emit(ctx, serde_json::to_value(&res).unwrap(), render_vat_enable);
            Ok(())
        }
        VatCmd::ListCodes => {
            let conn = ensure_db(ctx)?;
            let codes = vat::list_vat_codes(&conn)?;
            emit(ctx, json!({ "codes": serde_json::to_value(&codes).unwrap() }), render_vat_codes);
            Ok(())
        }
    }
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

fn render_vat_codes(_ctx: &Ctx, d: &Value) {
    for c in d["codes"].as_array().unwrap_or(&Vec::new()) {
        println!("{:<4} {}bp  {}", c["code"], c["rate_bp"], c["description"].as_str().unwrap_or(""));
    }
}
