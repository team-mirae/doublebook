//! One JSON request in, one JSON response out. Used by the C ABI and the CLI.
//!
//! Stateless: every request carries the book it works on. A request that
//! changes something returns the changed record for the caller to persist.
//!
//! Response: `{"ok": true, "result": ...}` or `{"ok": false, "error": {...}}`.

use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::book::{Change, Error, Index, validate_book, validate_entry};
use crate::migrate::terraform;
use crate::model::*;
use crate::reports;
use crate::time::{BusinessTime, BusinessTimeRange};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Request {
    #[serde(rename_all = "camelCase")]
    NewBook {
        unique_id: Option<Uuid>,
        name_english: String,
        name_korean: String,
        base_currency: Currency,
        closing_month: u8,
    },
    #[serde(rename_all = "camelCase")]
    UpdateBook {
        book: Book,
        header: Book,
    },
    ValidateBook {
        book: Book,
    },
    ValidateEntry {
        book: Book,
        entry: Entry,
    },
    CreateCurrency {
        book: Book,
        currency: Currency,
    },
    UpdateCurrency {
        book: Book,
        currency: Currency,
    },
    CreateAccount {
        book: Book,
        account: Account,
    },
    UpdateAccount {
        book: Book,
        account: Account,
    },
    CreateClient {
        book: Book,
        client: Client,
    },
    UpdateClient {
        book: Book,
        client: Client,
    },
    CreateEntry {
        book: Book,
        entry: Entry,
    },
    #[serde(rename_all = "camelCase")]
    UpdateEntry {
        book: Book,
        entry: Entry,
        author: String,
        reason: String,
        recorded_at: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    SetEntryState {
        book: Book,
        entry_id: Uuid,
        state: EntryState,
        author: String,
        reason: String,
        recorded_at: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    AccountBalance {
        book: Book,
        account_id: Uuid,
        at: BusinessTime,
        #[serde(default)]
        require_final: bool,
    },
    #[serde(rename_all = "camelCase")]
    TrialBalance {
        book: Book,
        at: BusinessTime,
        #[serde(default)]
        require_final: bool,
    },
    #[serde(rename_all = "camelCase")]
    BalanceSheet {
        book: Book,
        at: BusinessTime,
        #[serde(default)]
        require_final: bool,
    },
    #[serde(rename_all = "camelCase")]
    IncomeStatement {
        book: Book,
        range: BusinessTimeRange,
        #[serde(default)]
        require_final: bool,
    },
    #[serde(rename_all = "camelCase")]
    CashFlowStatement {
        book: Book,
        range: BusinessTimeRange,
        #[serde(default)]
        require_final: bool,
    },
    #[serde(rename_all = "camelCase")]
    ChangesInEquity {
        book: Book,
        range: BusinessTimeRange,
        #[serde(default)]
        require_final: bool,
    },
    #[serde(rename_all = "camelCase")]
    ClientBalances {
        book: Book,
        at: BusinessTime,
        #[serde(default)]
        require_final: bool,
        account_id: Option<Uuid>,
    },
    #[serde(rename_all = "camelCase")]
    GeneralLedger {
        book: Book,
        account_id: Uuid,
        range: BusinessTimeRange,
        #[serde(default)]
        require_final: bool,
    },
    #[serde(rename_all = "camelCase")]
    Journal {
        book: Book,
        range: BusinessTimeRange,
        #[serde(default)]
        require_final: bool,
    },
    #[serde(rename_all = "camelCase")]
    LegTagSummary {
        book: Book,
        range: BusinessTimeRange,
        #[serde(default)]
        require_final: bool,
    },
    FiscalYear {
        book: Book,
        year: i32,
    },
    /// Converts a Terraform accountant export (one array per table) into books.
    MigrateTerraform {
        export: terraform::Export,
    },
}

fn ok<T: serde::Serialize>(v: T) -> Result<Value, Error> {
    serde_json::to_value(v).map_err(|e| Error::BadRequest { message: e.to_string() })
}

pub fn handle(req: Request) -> Result<Value, Error> {
    use Request::*;
    match req {
        NewBook {
            unique_id,
            name_english,
            name_korean,
            base_currency,
            closing_month,
        } => ok(Book::new(unique_id, name_english, name_korean, base_currency, closing_month)?),
        UpdateBook { mut book, header } => {
            book.update_header(header)?;
            book.currencies.clear();
            book.accounts.clear();
            book.clients.clear();
            book.entries.clear();
            ok(book)
        }
        ValidateBook { book } => ok(validate_book(&book)),
        ValidateEntry { book, entry } => {
            let idx = Index::new(&book);
            let skip = idx.entry(entry.unique_id).map(|e| e.unique_id);
            ok(validate_entry(&idx, &entry, skip, true))
        }
        CreateCurrency { mut book, currency } => ok(book.create_currency(currency)?),
        UpdateCurrency { mut book, currency } => ok(book.update_currency(currency)?),
        CreateAccount { mut book, account } => ok(book.create_account(account)?),
        UpdateAccount { mut book, account } => ok(book.update_account(account)?),
        CreateClient { mut book, client } => ok(book.create_client(client)?),
        UpdateClient { mut book, client } => ok(book.update_client(client)?),
        CreateEntry { mut book, entry } => ok(book.create_entry(entry)?),
        UpdateEntry {
            mut book,
            entry,
            author,
            reason,
            recorded_at,
        } => ok(book.update_entry(
            entry,
            Change {
                author,
                reason,
                recorded_at,
            },
        )?),
        SetEntryState {
            mut book,
            entry_id,
            state,
            author,
            reason,
            recorded_at,
        } => ok(book.set_entry_state(
            entry_id,
            state,
            Change {
                author,
                reason,
                recorded_at,
            },
        )?),
        AccountBalance {
            book,
            account_id,
            at,
            require_final,
        } => ok(reports::account_balance(&book, account_id, at, require_final)?),
        TrialBalance { book, at, require_final } => ok(reports::trial_balance(&book, at, require_final)?),
        BalanceSheet { book, at, require_final } => ok(reports::balance_sheet(&book, at, require_final)?),
        IncomeStatement {
            book,
            range,
            require_final,
        } => ok(reports::income_statement(&book, range, require_final)?),
        CashFlowStatement {
            book,
            range,
            require_final,
        } => ok(reports::cash_flow_statement(&book, range, require_final)?),
        ChangesInEquity {
            book,
            range,
            require_final,
        } => ok(reports::changes_in_equity(&book, range, require_final)?),
        ClientBalances {
            book,
            at,
            require_final,
            account_id,
        } => ok(reports::client_balances(&book, at, require_final, account_id)?),
        GeneralLedger {
            book,
            account_id,
            range,
            require_final,
        } => ok(reports::general_ledger(&book, account_id, range, require_final)?),
        Journal {
            book,
            range,
            require_final,
        } => ok(reports::journal(&book, range, require_final)?),
        LegTagSummary {
            book,
            range,
            require_final,
        } => ok(reports::leg_tag_summary(&book, range, require_final)?),
        FiscalYear { book, year } => ok(reports::fiscal_year(&book, year)?),
        MigrateTerraform { export } => ok(terraform::migrate(&export)),
    }
}

/// Parses, handles and wraps. Never panics on bad input.
pub fn call(request: &str) -> String {
    let result = match serde_json::from_str::<Request>(request) {
        Ok(req) => handle(req),
        Err(e) => Err(Error::BadRequest { message: e.to_string() }),
    };
    let body = match result {
        Ok(v) => json!({"ok": true, "result": v}),
        Err(e) => json!({"ok": false, "error": e}),
    };
    body.to_string()
}
