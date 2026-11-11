//! Validation and the operations that change a book.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use uuid::Uuid;

use crate::model::*;
use crate::time::{BusinessTime, is_valid_at, now_rfc3339};

/// One thing wrong with a request. Every problem is reported at once.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leg: Option<usize>,
}

impl Problem {
    pub fn new(code: &str, message: impl Into<String>) -> Problem {
        Problem {
            code: code.to_string(),
            message: message.into(),
            entry_id: None,
            leg: None,
        }
    }

    fn leg(mut self, i: usize) -> Problem {
        self.leg = Some(i);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Error {
    /// The request would break a rule; nothing changed.
    #[serde(rename_all = "camelCase")]
    Invalid { problems: Vec<Problem> },
    #[serde(rename_all = "camelCase")]
    NotFound { what: String, id: String },
    /// requireFinal was set and these drafts are in scope.
    #[serde(rename_all = "camelCase")]
    DraftInScope { entry_ids: Vec<Uuid> },
    #[serde(rename_all = "camelCase")]
    BadRequest { message: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Invalid { problems } => {
                write!(f, "invalid:")?;
                for p in problems {
                    write!(f, " [{}] {};", p.code, p.message)?;
                }
                Ok(())
            }
            Error::NotFound { what, id } => write!(f, "{what} not found: {id}"),
            Error::DraftInScope { entry_ids } => write!(f, "{} draft entries in scope", entry_ids.len()),
            Error::BadRequest { message } => write!(f, "bad request: {message}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn invalid<T>(problems: Vec<Problem>) -> Result<T> {
    Err(Error::Invalid { problems })
}

fn not_found<T>(what: &str, id: impl ToString) -> Result<T> {
    Err(Error::NotFound {
        what: what.to_string(),
        id: id.to_string(),
    })
}

/// Lookup tables over a book, built once per operation.
pub struct Index<'a> {
    pub book: &'a Book,
    accounts: HashMap<Uuid, usize>,
    clients: HashMap<Uuid, usize>,
    entries: HashMap<Uuid, usize>,
    children: HashMap<Uuid, Vec<Uuid>>,
    roots: Vec<Uuid>,
    /// Accounts that have legs in any entry, voided included.
    with_legs: HashSet<Uuid>,
}

impl<'a> Index<'a> {
    pub fn new(book: &'a Book) -> Index<'a> {
        let accounts: HashMap<Uuid, usize> = book.accounts.iter().enumerate().map(|(i, a)| (a.unique_id, i)).collect();
        let mut children: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        let mut roots = Vec::new();
        for a in &book.accounts {
            match a.parent_id {
                Some(p) if accounts.contains_key(&p) => children.entry(p).or_default().push(a.unique_id),
                _ => roots.push(a.unique_id),
            }
        }
        let key = |id: &Uuid| {
            let a = &book.accounts[accounts[id]];
            (a.kind.report_rank(), a.sort_order, a.unique_id)
        };
        roots.sort_by_key(key);
        for list in children.values_mut() {
            list.sort_by_key(key);
        }
        let with_legs = book.entries.iter().flat_map(|e| e.legs.iter().map(|l| l.account_id)).collect();
        Index {
            book,
            accounts,
            clients: book.clients.iter().enumerate().map(|(i, c)| (c.unique_id, i)).collect(),
            entries: book.entries.iter().enumerate().map(|(i, e)| (e.unique_id, i)).collect(),
            children,
            roots,
            with_legs,
        }
    }

    pub fn account(&self, id: Uuid) -> Option<&'a Account> {
        self.accounts.get(&id).map(|&i| &self.book.accounts[i])
    }

    pub fn client(&self, id: Uuid) -> Option<&'a Client> {
        self.clients.get(&id).map(|&i| &self.book.clients[i])
    }

    pub fn entry(&self, id: Uuid) -> Option<&'a Entry> {
        self.entries.get(&id).map(|&i| &self.book.entries[i])
    }

    pub fn children(&self, id: Uuid) -> &[Uuid] {
        self.children.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Top-level accounts in tree order: type, then sortOrder, then uniqueId.
    pub fn roots(&self) -> &[Uuid] {
        &self.roots
    }

    pub fn is_leaf(&self, id: Uuid) -> bool {
        self.children(id).is_empty()
    }

    pub fn has_legs(&self, id: Uuid) -> bool {
        self.with_legs.contains(&id)
    }

    /// The account and its ancestors, nearest first; stops at a cycle.
    pub fn chain(&self, id: Uuid) -> Vec<&'a Account> {
        let mut out: Vec<&Account> = Vec::new();
        let mut cur = self.account(id);
        while let Some(a) = cur {
            if out.iter().any(|x| x.unique_id == a.unique_id) {
                break;
            }
            out.push(a);
            cur = a.parent_id.and_then(|p| self.account(p));
        }
        out
    }

    /// The account and everything under it, in tree order.
    pub fn subtree(&self, id: Uuid) -> Vec<Uuid> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        self.walk(id, &mut out, &mut seen);
        out
    }

    fn walk(&self, id: Uuid, out: &mut Vec<Uuid>, seen: &mut HashSet<Uuid>) {
        if !seen.insert(id) {
            return;
        }
        out.push(id);
        for &c in self.children(id) {
            self.walk(c, out, seen);
        }
    }

    /// Every account in tree order.
    pub fn tree_order(&self) -> Vec<Uuid> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for &r in &self.roots {
            self.walk(r, &mut out, &mut seen);
        }
        out
    }

    /// Valid at `t`, together with every ancestor.
    pub fn account_valid_at(&self, id: Uuid, t: BusinessTime) -> bool {
        let chain = self.chain(id);
        !chain.is_empty() && chain.iter().all(|a| is_valid_at(a.valid_from, a.valid_until, &a.invalid_ranges, t))
    }

    pub fn client_valid_at(&self, id: Uuid, t: BusinessTime) -> bool {
        self.client(id)
            .is_some_and(|c| is_valid_at(c.valid_from, c.valid_until, &c.invalid_ranges, t))
    }

    pub fn has_currency(&self, code: &str) -> bool {
        self.book.currencies.iter().any(|c| c.code == code)
    }
}

/// Entries sorted by time, then uuidv7.
pub fn ordered_entries(book: &Book) -> Vec<&Entry> {
    let mut list: Vec<&Entry> = book.entries.iter().collect();
    list.sort_by(|a, b| a.time.cmp(&b.time).then(a.unique_id.cmp(&b.unique_id)));
    list
}

fn zero() -> Dec {
    Dec::from(0)
}

/// Rules 1-10, 12, 14 and 15 for one entry. Voided entries are not checked.
/// `skip` excludes an entry id from rule 14 (the stored version of an entry
/// being updated). `draft_side` also checks rule 14 from the draft's side.
pub fn validate_entry(idx: &Index, entry: &Entry, skip: Option<Uuid>, draft_side: bool) -> Vec<Problem> {
    let mut p = Vec::new();
    if entry.state == EntryState::Voided {
        return p;
    }
    if entry.legs.len() < 2 {
        p.push(Problem::new("tooFewLegs", "an entry has at least two legs"));
    }
    let (mut debit, mut credit, mut cash_debit, mut cash_credit) = (zero(), zero(), zero(), zero());
    for (i, leg) in entry.legs.iter().enumerate() {
        match leg.side {
            LegSide::Debit => {
                debit += &leg.book_amount;
                cash_debit += &leg.cash_amount;
            }
            LegSide::Credit => {
                credit += &leg.book_amount;
                cash_credit += &leg.cash_amount;
            }
        }
        if leg.book_amount < zero() {
            p.push(Problem::new("negativeAmount", "bookAmount must be zero or positive").leg(i));
        }
        if leg.original_amount < zero() {
            p.push(Problem::new("negativeAmount", "originalAmount must be zero or positive").leg(i));
        }
        if leg.cash_amount < zero() || leg.cash_amount > leg.book_amount {
            p.push(Problem::new("cashAmountOutOfRange", "cashAmount must be within [0, bookAmount]").leg(i));
        }
        let Some(account) = idx.account(leg.account_id) else {
            p.push(Problem::new("unknownAccount", format!("account {} does not exist", leg.account_id)).leg(i));
            continue;
        };
        if !idx.is_leaf(account.unique_id) {
            p.push(
                Problem::new(
                    "notLeaf",
                    format!("account {} has children; only leaves take legs", account.unique_id),
                )
                .leg(i),
            );
        }
        if !idx.account_valid_at(account.unique_id, entry.time) {
            p.push(
                Problem::new(
                    "accountNotValid",
                    format!("account {} is not valid at {}", account.unique_id, entry.time),
                )
                .leg(i),
            );
        }
        if leg.original_currency != account.base_currency {
            p.push(
                Problem::new(
                    "currencyMismatch",
                    format!(
                        "originalCurrency {} differs from the account's {}",
                        leg.original_currency, account.base_currency
                    ),
                )
                .leg(i),
            );
        }
        if !idx.has_currency(&leg.original_currency) {
            p.push(Problem::new("unknownCurrency", format!("currency {} is not in the book", leg.original_currency)).leg(i));
        }
        if account.base_currency == idx.book.base_currency && leg.original_amount != leg.book_amount {
            p.push(Problem::new("baseAmountMismatch", "in a base-currency account originalAmount equals bookAmount").leg(i));
        }
        if let Some(c) = leg.client_id {
            if idx.client(c).is_none() {
                p.push(Problem::new("unknownClient", format!("client {c} does not exist")).leg(i));
            } else if !idx.client_valid_at(c, entry.time) {
                p.push(Problem::new("clientNotValid", format!("client {c} is not valid at {}", entry.time)).leg(i));
            }
        }
        if account.is_cash_or_cash_equivalent {
            if leg.cash_amount != leg.book_amount {
                p.push(Problem::new("cashLegNotFull", "on a cash account cashAmount equals bookAmount").leg(i));
            }
            if leg.metadata.kind.is_some() {
                p.push(Problem::new("cashLegHasType", "a leg on a cash account has no LegType").leg(i));
            }
        } else if leg.metadata.kind.is_none() {
            p.push(Problem::new("missingLegType", "a leg on a non-cash account needs a LegType").leg(i));
        }
    }
    if debit != credit {
        p.push(Problem::new(
            "unbalanced",
            format!(
                "debits {} differ from credits {}",
                debit.to_plain_string(),
                credit.to_plain_string()
            ),
        ));
    }
    if cash_debit != cash_credit {
        p.push(Problem::new(
            "cashUnbalanced",
            format!(
                "debit cashAmount {} differs from credit cashAmount {}",
                cash_debit.to_plain_string(),
                cash_credit.to_plain_string()
            ),
        ));
    }
    let others = idx
        .book
        .entries
        .iter()
        .filter(|e| Some(e.unique_id) != skip && e.unique_id != entry.unique_id);
    if entry.kind == EntryType::Closing {
        let drafts: Vec<String> = others
            .clone()
            .filter(|e| e.state == EntryState::Draft && e.time <= entry.time)
            .map(|e| e.unique_id.to_string())
            .collect();
        if !drafts.is_empty() {
            p.push(Problem::new(
                "draftBeforeClosing",
                format!("drafts at or before this closing entry: {}", drafts.join(", ")),
            ));
        }
    }
    if draft_side && entry.state == EntryState::Draft {
        let closings: Vec<String> = others
            .filter(|e| e.kind == EntryType::Closing && e.state != EntryState::Voided && e.time >= entry.time)
            .map(|e| e.unique_id.to_string())
            .collect();
        if !closings.is_empty() {
            p.push(Problem::new(
                "draftBeforeClosing",
                format!("a closing entry is at or after this draft: {}", closings.join(", ")),
            ));
        }
    }
    for x in &mut p {
        x.entry_id = Some(entry.unique_id);
    }
    p
}

/// Rules on one account: tree, currency, contraOf (11), cash flag (13), validity window.
pub fn validate_account(idx: &Index, a: &Account) -> Vec<Problem> {
    let mut p = Vec::new();
    if let Some(parent) = a.parent_id {
        match idx.account(parent) {
            None => p.push(Problem::new("unknownParent", format!("parent {parent} does not exist"))),
            Some(pa) if pa.kind != a.kind => p.push(Problem::new("parentType", "a parent has the same type as its child")),
            Some(_) => {
                if parent == a.unique_id || idx.chain(parent).iter().any(|x| x.unique_id == a.unique_id) {
                    p.push(Problem::new("cycle", "the account tree must not contain cycles"));
                }
            }
        }
    }
    if !idx.has_currency(&a.base_currency) {
        p.push(Problem::new(
            "unknownCurrency",
            format!("currency {} is not in the book", a.base_currency),
        ));
    }
    if let Some(c) = a.contra_of {
        match idx.account(c) {
            None => p.push(Problem::new("unknownContra", format!("contraOf {c} does not exist"))),
            Some(_) if c == a.unique_id => p.push(Problem::new("contraSelf", "an account cannot be a contra of itself")),
            Some(ca) if ca.kind != a.kind => p.push(Problem::new("contraType", "contraOf names an account of the same type")),
            Some(_) => {}
        }
    }
    if a.is_cash_or_cash_equivalent && a.kind != AccountType::Asset {
        p.push(Problem::new(
            "cashNotAsset",
            "isCashOrCashEquivalent is only true on asset accounts",
        ));
    }
    if a.valid_until.is_some_and(|u| u < a.valid_from) {
        p.push(Problem::new("validity", "validUntil precedes validFrom"));
    }
    if a.invalid_ranges.iter().any(|r| r.until < r.from) {
        p.push(Problem::new("validity", "an invalid range ends before it starts"));
    }
    p
}

fn validate_client(c: &Client) -> Vec<Problem> {
    let mut p = Vec::new();
    if c.valid_until.is_some_and(|u| u < c.valid_from) {
        p.push(Problem::new("validity", "validUntil precedes validFrom"));
    }
    if c.invalid_ranges.iter().any(|r| r.until < r.from) {
        p.push(Problem::new("validity", "an invalid range ends before it starts"));
    }
    p
}

fn validate_currency(c: &Currency) -> Vec<Problem> {
    let mut p = Vec::new();
    if !(2..=12).contains(&c.code.len()) || !c.code.bytes().all(|b| b.is_ascii_alphanumeric()) {
        p.push(Problem::new(
            "currencyCode",
            format!("currency code \"{}\" must be 2-12 ASCII letters or digits", c.code),
        ));
    }
    if c.display_decimals > 12 {
        p.push(Problem::new("displayDecimals", "displayDecimals is within [0, 12]"));
    }
    p
}

/// Every rule over a whole book. Empty means valid.
pub fn validate_book(book: &Book) -> Vec<Problem> {
    let idx = Index::new(book);
    let mut p = Vec::new();
    if !(1..=12).contains(&book.closing_month) {
        p.push(Problem::new("closingMonth", "closingMonth is within [1, 12]"));
    }
    if !idx.has_currency(&book.base_currency) {
        p.push(Problem::new(
            "unknownCurrency",
            format!("base currency {} is not in the book", book.base_currency),
        ));
    }
    let mut codes = HashSet::new();
    for c in &book.currencies {
        p.extend(validate_currency(c));
        if !codes.insert(c.code.as_str()) {
            p.push(Problem::new("duplicate", format!("currency {} appears twice", c.code)));
        }
    }
    let mut ids = HashSet::new();
    for a in &book.accounts {
        if !ids.insert(a.unique_id) {
            p.push(Problem::new("duplicate", format!("account {} appears twice", a.unique_id)));
        }
        for mut x in validate_account(&idx, a) {
            x.message = format!("account {}: {}", a.unique_id, x.message);
            p.push(x);
        }
        if !idx.is_leaf(a.unique_id) && idx.has_legs(a.unique_id) {
            p.push(Problem::new(
                "notLeaf",
                format!("account {} has both children and legs", a.unique_id),
            ));
        }
    }
    for c in &book.clients {
        if !ids.insert(c.unique_id) {
            p.push(Problem::new("duplicate", format!("client {} appears twice", c.unique_id)));
        }
        for mut x in validate_client(c) {
            x.message = format!("client {}: {}", c.unique_id, x.message);
            p.push(x);
        }
    }
    for e in &book.entries {
        if !ids.insert(e.unique_id) {
            p.push(Problem::new("duplicate", format!("entry {} appears twice", e.unique_id)));
        }
        if e.unique_id.get_version_num() != 7 {
            p.push(Problem::new("entryIdVersion", format!("entry {} is not a uuidv7", e.unique_id)));
        }
        for (i, r) in e.revisions.iter().enumerate() {
            if r.revision as usize != i + 1 {
                let mut x = Problem::new("revisionNumber", "revisions are numbered 1, 2, 3, ...");
                x.entry_id = Some(e.unique_id);
                p.push(x);
                break;
            }
        }
        p.extend(validate_entry(&idx, e, None, false));
    }
    p
}

fn check_leg_validity(idx: &Index, accounts: &HashSet<Uuid>, clients: &HashSet<Uuid>) -> Vec<Problem> {
    let mut p = Vec::new();
    for e in &idx.book.entries {
        if e.state == EntryState::Voided {
            continue;
        }
        for (i, leg) in e.legs.iter().enumerate() {
            if accounts.contains(&leg.account_id) && !idx.account_valid_at(leg.account_id, e.time) {
                let mut x = Problem::new(
                    "legOutsideValidity",
                    format!("account {} would not be valid at {}", leg.account_id, e.time),
                )
                .leg(i);
                x.entry_id = Some(e.unique_id);
                p.push(x);
            }
            if let Some(c) = leg.client_id
                && clients.contains(&c)
                && !idx.client_valid_at(c, e.time)
            {
                let mut x = Problem::new("legOutsideValidity", format!("client {c} would not be valid at {}", e.time)).leg(i);
                x.entry_id = Some(e.unique_id);
                p.push(x);
            }
        }
    }
    p
}

/// Who and when, for a change that records a revision.
#[derive(Clone, Debug, Default)]
pub struct Change {
    pub author: String,
    pub reason: String,
    /// RFC 3339; now when absent.
    pub recorded_at: Option<String>,
}

impl Book {
    pub fn new(
        unique_id: Option<Uuid>,
        name_english: String,
        name_korean: String,
        base_currency: Currency,
        closing_month: u8,
    ) -> Result<Book> {
        let book = Book {
            unique_id: unique_id.unwrap_or_else(Uuid::new_v4),
            legacy_id: None,
            name_english,
            name_korean,
            base_currency: base_currency.code.clone(),
            closing_month,
            currencies: vec![base_currency],
            accounts: vec![],
            clients: vec![],
            entries: vec![],
            extra: Extra::new(),
        };
        let p = validate_book(&book);
        if p.is_empty() { Ok(book) } else { invalid(p) }
    }

    /// Changes names, closingMonth and extra. baseCurrency may change only while the book has no entries.
    pub fn update_header(&mut self, header: Book) -> Result<()> {
        let mut p = Vec::new();
        if header.base_currency != self.base_currency {
            if !self.entries.is_empty() {
                p.push(Problem::new(
                    "baseCurrencyLocked",
                    "baseCurrency cannot change once the book has entries",
                ));
            } else if !self.currencies.iter().any(|c| c.code == header.base_currency) {
                p.push(Problem::new(
                    "unknownCurrency",
                    format!("currency {} is not in the book", header.base_currency),
                ));
            }
        }
        if !(1..=12).contains(&header.closing_month) {
            p.push(Problem::new("closingMonth", "closingMonth is within [1, 12]"));
        }
        if !p.is_empty() {
            return invalid(p);
        }
        self.name_english = header.name_english;
        self.name_korean = header.name_korean;
        self.base_currency = header.base_currency;
        self.closing_month = header.closing_month;
        self.extra = header.extra;
        Ok(())
    }

    pub fn create_currency(&mut self, c: Currency) -> Result<Currency> {
        let mut p = validate_currency(&c);
        if self.currencies.iter().any(|x| x.code == c.code) {
            p.push(Problem::new("duplicate", format!("currency {} already exists", c.code)));
        }
        if !p.is_empty() {
            return invalid(p);
        }
        self.currencies.push(c.clone());
        Ok(c)
    }

    /// The code identifies the currency and never changes.
    pub fn update_currency(&mut self, c: Currency) -> Result<Currency> {
        let p = validate_currency(&c);
        if !p.is_empty() {
            return invalid(p);
        }
        let Some(slot) = self.currencies.iter_mut().find(|x| x.code == c.code) else {
            return not_found("currency", &c.code);
        };
        *slot = c.clone();
        Ok(c)
    }

    pub fn create_account(&mut self, mut a: Account) -> Result<Account> {
        if a.unique_id.is_nil() {
            a.unique_id = Uuid::new_v4();
        }
        let idx = Index::new(self);
        let mut p = validate_account(&idx, &a);
        if idx.account(a.unique_id).is_some() || idx.client(a.unique_id).is_some() {
            p.push(Problem::new("duplicate", format!("id {} already exists", a.unique_id)));
        }
        if let Some(parent) = a.parent_id
            && idx.has_legs(parent)
        {
            p.push(Problem::new("parentHasLegs", "an account that has legs cannot be given a child"));
        }
        if !p.is_empty() {
            return invalid(p);
        }
        self.accounts.push(a.clone());
        Ok(a)
    }

    /// Full replacement of an account, identified by uniqueId.
    pub fn update_account(&mut self, a: Account) -> Result<Account> {
        let Some(pos) = self.accounts.iter().position(|x| x.unique_id == a.unique_id) else {
            return not_found("account", a.unique_id);
        };
        let old = self.accounts[pos].clone();
        let mut p = Vec::new();
        {
            let idx = Index::new(self);
            let used = idx.has_legs(a.unique_id);
            if a.kind != old.kind {
                p.push(Problem::new("typeLocked", "type is immutable once created"));
            }
            if used && a.base_currency != old.base_currency {
                p.push(Problem::new(
                    "currencyLocked",
                    "baseCurrency is immutable once the account has legs",
                ));
            }
            if used && a.is_cash_or_cash_equivalent != old.is_cash_or_cash_equivalent {
                p.push(Problem::new(
                    "cashFlagLocked",
                    "isCashOrCashEquivalent is immutable once the account has legs",
                ));
            }
            if a.parent_id != old.parent_id
                && let Some(parent) = a.parent_id
                && idx.has_legs(parent)
            {
                p.push(Problem::new("parentHasLegs", "an account that has legs cannot be given a child"));
            }
        }
        if !p.is_empty() {
            return invalid(p);
        }
        self.accounts[pos] = a.clone();
        let idx = Index::new(self);
        let mut p = validate_account(&idx, &a);
        if p.is_empty() {
            let subtree: HashSet<Uuid> = idx.subtree(a.unique_id).into_iter().collect();
            p.extend(check_leg_validity(&idx, &subtree, &HashSet::new()));
        }
        if !p.is_empty() {
            self.accounts[pos] = old;
            return invalid(p);
        }
        Ok(a)
    }

    pub fn create_client(&mut self, mut c: Client) -> Result<Client> {
        if c.unique_id.is_nil() {
            c.unique_id = Uuid::new_v4();
        }
        let mut p = validate_client(&c);
        let idx = Index::new(self);
        if idx.account(c.unique_id).is_some() || idx.client(c.unique_id).is_some() {
            p.push(Problem::new("duplicate", format!("id {} already exists", c.unique_id)));
        }
        if !p.is_empty() {
            return invalid(p);
        }
        self.clients.push(c.clone());
        Ok(c)
    }

    pub fn update_client(&mut self, c: Client) -> Result<Client> {
        let p = validate_client(&c);
        if !p.is_empty() {
            return invalid(p);
        }
        let Some(pos) = self.clients.iter().position(|x| x.unique_id == c.unique_id) else {
            return not_found("client", c.unique_id);
        };
        let old = std::mem::replace(&mut self.clients[pos], c.clone());
        let idx = Index::new(self);
        let p = check_leg_validity(&idx, &HashSet::new(), &HashSet::from([c.unique_id]));
        if !p.is_empty() {
            self.clients[pos] = old;
            return invalid(p);
        }
        Ok(c)
    }

    /// Posts a new entry. A nil uniqueId gets a fresh uuidv7.
    pub fn create_entry(&mut self, mut e: Entry) -> Result<Entry> {
        if e.unique_id.is_nil() {
            e.unique_id = Uuid::now_v7();
        }
        let idx = Index::new(self);
        let mut p = Vec::new();
        if e.unique_id.get_version_num() != 7 {
            p.push(Problem::new("entryIdVersion", "an entry's uniqueId is a uuidv7"));
        }
        if idx.entry(e.unique_id).is_some() {
            p.push(Problem::new("duplicate", format!("entry {} already exists", e.unique_id)));
        }
        if !e.revisions.is_empty() {
            p.push(Problem::new("revisions", "a new entry has no revisions"));
        }
        p.extend(validate_entry(&idx, &e, None, true));
        if !p.is_empty() {
            return invalid(p);
        }
        self.entries.push(e.clone());
        Ok(e)
    }

    /// Replaces an entry's content (state included) and appends a revision
    /// holding the entry as it was. uniqueId, legacyId and revisions are kept.
    pub fn update_entry(&mut self, mut e: Entry, change: Change) -> Result<Entry> {
        let Some(pos) = self.entries.iter().position(|x| x.unique_id == e.unique_id) else {
            return not_found("entry", e.unique_id);
        };
        let mut p = Vec::new();
        if change.reason.trim().is_empty() {
            p.push(Problem::new("reasonRequired", "a change to an entry needs a reason"));
        }
        let idx = Index::new(self);
        p.extend(validate_entry(&idx, &e, Some(e.unique_id), true));
        if !p.is_empty() {
            return invalid(p);
        }
        let old = &mut self.entries[pos];
        let mut before = old.clone();
        before.revisions.clear();
        e.legacy_id = old.legacy_id.clone();
        e.revisions = std::mem::take(&mut old.revisions);
        e.revisions.push(EntryRevision {
            revision: e.revisions.len() as u32 + 1,
            recorded_at: change.recorded_at.unwrap_or_else(now_rfc3339),
            author: change.author,
            reason: change.reason,
            before: Box::new(before),
        });
        *old = e.clone();
        Ok(e)
    }

    /// Convenience: change only the state (draft, final, voided and back).
    pub fn set_entry_state(&mut self, id: Uuid, state: EntryState, change: Change) -> Result<Entry> {
        let Some(e) = self.entries.iter().find(|x| x.unique_id == id) else {
            return not_found("entry", id);
        };
        let mut e = e.clone();
        e.state = state;
        self.update_entry(e, change)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::str::FromStr;

    pub fn t(s: &str) -> BusinessTime {
        s.parse().unwrap()
    }

    pub fn d(s: &str) -> Dec {
        Dec::from_str(s).unwrap()
    }

    pub fn krw() -> Currency {
        Currency {
            code: "KRW".into(),
            name_english: "Korean won".into(),
            name_korean: "원".into(),
            symbol: None,
            display_decimals: 0,
            extra: Extra::new(),
        }
    }

    pub fn account(kind: AccountType, name: &str, parent: Option<Uuid>, cash: bool) -> Account {
        Account {
            unique_id: Uuid::new_v4(),
            legacy_id: None,
            kind,
            parent_id: parent,
            sort_order: 0,
            name_english: name.into(),
            name_korean: name.into(),
            valid_from: BusinessTime::MIN,
            valid_until: None,
            invalid_ranges: vec![],
            base_currency: "KRW".into(),
            contra_of: None,
            is_cash_or_cash_equivalent: cash,
            tags: vec![],
            extra: Extra::new(),
        }
    }

    pub fn leg(account: &Account, side: LegSide, amount: &str, cash: &str, kind: Option<LegType>) -> EntryLeg {
        EntryLeg {
            account_id: account.unique_id,
            side,
            client_id: None,
            book_amount: d(amount),
            cash_amount: d(cash),
            original_amount: d(amount),
            original_currency: "KRW".into(),
            is_tax_relevant: matches!(account.kind, AccountType::Income | AccountType::Expense),
            metadata: LegMetadata { kind, tags: vec![] },
            extra: Extra::new(),
        }
    }

    pub fn entry(time: &str, legs: Vec<EntryLeg>) -> Entry {
        Entry {
            unique_id: Uuid::nil(),
            legacy_id: None,
            name: String::new(),
            description: String::new(),
            remarks: String::new(),
            time: t(time),
            legs,
            kind: EntryType::Normal,
            state: EntryState::Final,
            revisions: vec![],
            extra: Extra::new(),
        }
    }

    pub struct Fixture {
        pub book: Book,
        pub bank: Account,
        pub supplies: Account,
        pub revenue: Account,
        pub loan: Account,
        pub capital: Account,
    }

    pub fn fixture() -> Fixture {
        let mut book = Book::new(None, "Test".into(), "테스트".into(), krw(), 12).unwrap();
        let bank = book.create_account(account(AccountType::Asset, "Bank", None, true)).unwrap();
        let supplies = book.create_account(account(AccountType::Expense, "Supplies", None, false)).unwrap();
        let revenue = book.create_account(account(AccountType::Income, "Revenue", None, false)).unwrap();
        let loan = book.create_account(account(AccountType::Liability, "Loan", None, false)).unwrap();
        let capital = book.create_account(account(AccountType::Equity, "Capital", None, false)).unwrap();
        Fixture {
            book,
            bank,
            supplies,
            revenue,
            loan,
            capital,
        }
    }

    fn codes(r: Result<Entry>) -> Vec<String> {
        match r {
            Err(Error::Invalid { problems }) => problems.into_iter().map(|p| p.code).collect(),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn posts_a_balanced_entry() {
        let mut f = fixture();
        let e = f
            .book
            .create_entry(entry(
                "2026-01-02T10:00",
                vec![
                    leg(&f.bank, LegSide::Debit, "150", "150", None),
                    leg(&f.loan, LegSide::Credit, "100", "100", Some(LegType::Financing)),
                    leg(&f.revenue, LegSide::Credit, "50", "50", Some(LegType::Operating)),
                ],
            ))
            .unwrap();
        assert_eq!(e.unique_id.get_version_num(), 7);
        assert!(validate_book(&f.book).is_empty());
    }

    #[test]
    fn zero_entry_is_allowed() {
        let mut f = fixture();
        f.book
            .create_entry(entry(
                "2026-01-02T10:00",
                vec![
                    leg(&f.bank, LegSide::Debit, "0", "0", None),
                    leg(&f.revenue, LegSide::Credit, "0", "0", Some(LegType::Operating)),
                ],
            ))
            .unwrap();
    }

    #[test]
    fn reports_every_problem() {
        let mut f = fixture();
        let c = codes(f.book.create_entry(entry(
            "2026-01-02T10:00",
            vec![
                leg(&f.bank, LegSide::Debit, "100", "90", Some(LegType::Operating)),
                leg(&f.revenue, LegSide::Credit, "90", "100", None),
            ],
        )));
        for code in [
            "cashLegNotFull",
            "cashLegHasType",
            "cashAmountOutOfRange",
            "missingLegType",
            "unbalanced",
            "cashUnbalanced",
        ] {
            assert!(c.contains(&code.to_string()), "{code} missing from {c:?}");
        }
    }

    #[test]
    fn closing_rejected_after_draft() {
        let mut f = fixture();
        let mut draft = entry(
            "2026-12-01T10:00",
            vec![
                leg(&f.bank, LegSide::Debit, "10", "10", None),
                leg(&f.revenue, LegSide::Credit, "10", "10", Some(LegType::Operating)),
            ],
        );
        draft.state = EntryState::Draft;
        let draft = f.book.create_entry(draft).unwrap();
        let mut closing = entry(
            "2026-12-31T99:00",
            vec![
                leg(&f.revenue, LegSide::Debit, "10", "0", Some(LegType::Other)),
                leg(&f.capital, LegSide::Credit, "10", "0", Some(LegType::Other)),
            ],
        );
        closing.kind = EntryType::Closing;
        assert!(codes(f.book.create_entry(closing.clone())).contains(&"draftBeforeClosing".to_string()));
        let change = Change {
            author: "t".into(),
            reason: "done".into(),
            recorded_at: None,
        };
        f.book.set_entry_state(draft.unique_id, EntryState::Final, change.clone()).unwrap();
        f.book.create_entry(closing).unwrap();
        assert!(codes(f.book.set_entry_state(draft.unique_id, EntryState::Draft, change)).contains(&"draftBeforeClosing".to_string()));
    }

    #[test]
    fn revisions_are_logged() {
        let mut f = fixture();
        let e = f
            .book
            .create_entry(entry(
                "2026-01-02T10:00",
                vec![
                    leg(&f.bank, LegSide::Debit, "10", "10", None),
                    leg(&f.revenue, LegSide::Credit, "10", "10", Some(LegType::Operating)),
                ],
            ))
            .unwrap();
        let change = Change {
            author: "me".into(),
            reason: "wrong".into(),
            recorded_at: Some("2026-11-12T00:00:00.000Z".into()),
        };
        let v = f.book.set_entry_state(e.unique_id, EntryState::Voided, change).unwrap();
        assert_eq!(v.revisions.len(), 1);
        assert_eq!(v.revisions[0].before.state, EntryState::Final);
        assert!(codes(f.book.set_entry_state(e.unique_id, EntryState::Final, Change::default())).contains(&"reasonRequired".to_string()));
    }

    #[test]
    fn retiring_checks_existing_legs() {
        let mut f = fixture();
        f.book
            .create_entry(entry(
                "2026-03-01T10:00",
                vec![
                    leg(&f.bank, LegSide::Debit, "10", "10", None),
                    leg(&f.revenue, LegSide::Credit, "10", "10", Some(LegType::Operating)),
                ],
            ))
            .unwrap();
        let mut r = f.revenue.clone();
        r.valid_until = Some(t("2026-02-01T00:00"));
        assert!(matches!(f.book.update_account(r.clone()), Err(Error::Invalid { .. })));
        r.valid_until = Some(t("2026-03-01T10:01"));
        f.book.update_account(r).unwrap();
        let mut b = f.bank.clone();
        b.is_cash_or_cash_equivalent = false;
        assert!(matches!(f.book.update_account(b), Err(Error::Invalid { .. })));
    }

    #[test]
    fn leaves_only() {
        let mut f = fixture();
        f.book
            .create_entry(entry(
                "2026-03-01T10:00",
                vec![
                    leg(&f.bank, LegSide::Debit, "10", "10", None),
                    leg(&f.revenue, LegSide::Credit, "10", "10", Some(LegType::Operating)),
                ],
            ))
            .unwrap();
        let child = account(AccountType::Asset, "Sub", Some(f.bank.unique_id), true);
        assert!(matches!(f.book.create_account(child), Err(Error::Invalid { .. })));
    }
}
