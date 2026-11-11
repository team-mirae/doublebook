//! The report functions of `docs/SPEC.md` § Reports.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;
use uuid::Uuid;

use crate::book::{Error, Index, Result, ordered_entries};
use crate::model::*;
use crate::time::{BusinessTime, BusinessTimeRange, Date};

fn zero() -> Dec {
    Dec::from(0)
}

fn is_zero(d: &Dec) -> bool {
    *d == zero()
}

/// The last moment a report reads.
#[derive(Clone, Copy)]
enum End {
    AtOrBefore(BusinessTime),
    Before(BusinessTime),
}

impl End {
    fn covers(self, t: BusinessTime) -> bool {
        match self {
            End::AtOrBefore(at) => t <= at,
            End::Before(until) => t < until,
        }
    }
}

/// Non-voided entries in order, after the requireFinal check.
fn scope(book: &Book, end: End, require_final: bool) -> Result<Vec<&Entry>> {
    let list: Vec<&Entry> = ordered_entries(book)
        .into_iter()
        .filter(|e| e.state != EntryState::Voided)
        .collect();
    if require_final {
        let drafts: Vec<Uuid> = list
            .iter()
            .filter(|e| e.state == EntryState::Draft && end.covers(e.time))
            .map(|e| e.unique_id)
            .collect();
        if !drafts.is_empty() {
            return Err(Error::DraftInScope { entry_ids: drafts });
        }
    }
    Ok(list)
}

fn range_ok(range: &BusinessTimeRange) -> Result<()> {
    if range.until < range.from {
        return Err(Error::BadRequest {
            message: "range.until precedes range.from".into(),
        });
    }
    Ok(())
}

#[derive(Clone, Default)]
struct Sums {
    natural: Dec,
    original: Dec,
}

/// Natural (and own-currency) balance per leaf over the legs that pass the filters.
fn leaf_sums<'a>(
    idx: &Index,
    entries: impl IntoIterator<Item = &'a Entry>,
    entry_filter: impl Fn(&Entry) -> bool,
    leg_filter: impl Fn(&EntryLeg) -> bool,
) -> HashMap<Uuid, Sums> {
    let mut out: HashMap<Uuid, Sums> = HashMap::new();
    for e in entries {
        if !entry_filter(e) {
            continue;
        }
        for leg in e.legs.iter().filter(|l| leg_filter(l)) {
            let Some(a) = idx.account(leg.account_id) else { continue };
            let s = out.entry(leg.account_id).or_default();
            s.natural += leg.natural(a.kind);
            s.original += natural(a.kind, leg.side, &leg.original_amount);
        }
    }
    out
}

fn nonzero_keys(sums: &HashMap<Uuid, Sums>) -> HashSet<Uuid> {
    sums.iter().filter(|(_, s)| !is_zero(&s.natural)).map(|(k, _)| *k).collect()
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountNode {
    pub account_id: Uuid,
    #[serde(with = "dec_str")]
    pub balance: Dec,
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "opt_dec")]
    pub original_balance: Option<Dec>,
    pub children: Vec<AccountNode>,
}

fn opt_dec<S: serde::Serializer>(d: &Option<Dec>, s: S) -> std::result::Result<S::Ok, S::Error> {
    match d {
        Some(d) => s.serialize_str(&d.to_plain_string()),
        None => s.serialize_none(),
    }
}

/// Builds the node for `id`: balance over the whole subtree, children limited to those that appear.
fn node(idx: &Index, id: Uuid, sums: &HashMap<Uuid, Sums>, appear: &HashSet<Uuid>, seen: &mut HashSet<Uuid>) -> (AccountNode, bool) {
    let mut balance = sums.get(&id).map(|s| s.natural.clone()).unwrap_or_else(zero);
    let mut shown = appear.contains(&id);
    let mut children = Vec::new();
    if seen.insert(id) {
        for &c in idx.children(id) {
            let (child, child_shown) = node(idx, c, sums, appear, seen);
            balance += &child.balance;
            if child_shown {
                shown = true;
                children.push(child);
            }
        }
    }
    let original_balance = idx
        .account(id)
        .filter(|a| idx.is_leaf(id) && a.base_currency != idx.book.base_currency)
        .map(|_| sums.get(&id).map(|s| s.original.clone()).unwrap_or_else(zero));
    (
        AccountNode {
            account_id: id,
            balance,
            original_balance,
            children,
        },
        shown,
    )
}

/// Appearing nodes for every root of one type.
fn nodes_of(idx: &Index, kind: AccountType, sums: &HashMap<Uuid, Sums>, appear: &HashSet<Uuid>) -> Vec<AccountNode> {
    let mut seen = HashSet::new();
    idx.roots()
        .iter()
        .filter(|&&r| idx.account(r).is_some_and(|a| a.kind == kind))
        .filter_map(|&r| {
            let (n, shown) = node(idx, r, sums, appear, &mut seen);
            shown.then_some(n)
        })
        .collect()
}

fn total_of(idx: &Index, kind: AccountType, sums: &HashMap<Uuid, Sums>) -> Dec {
    let mut t = zero();
    for (id, s) in sums {
        if idx.account(*id).is_some_and(|a| a.kind == kind) {
            t += &s.natural;
        }
    }
    t
}

pub fn account_balance(book: &Book, account_id: Uuid, at: BusinessTime, require_final: bool) -> Result<AccountNode> {
    let entries = scope(book, End::AtOrBefore(at), require_final)?;
    let idx = Index::new(book);
    if idx.account(account_id).is_none() {
        return Err(Error::NotFound {
            what: "account".into(),
            id: account_id.to_string(),
        });
    }
    let sums = leaf_sums(&idx, entries, |e| e.time <= at, |_| true);
    let appear = nonzero_keys(&sums);
    Ok(node(&idx, account_id, &sums, &appear, &mut HashSet::new()).0)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialBalance {
    pub rows: Vec<TrialBalanceRow>,
    #[serde(with = "dec_str")]
    pub total_debit: Dec,
    #[serde(with = "dec_str")]
    pub total_credit: Dec,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialBalanceRow {
    pub account_id: Uuid,
    #[serde(with = "dec_str")]
    pub debit: Dec,
    #[serde(with = "dec_str")]
    pub credit: Dec,
    #[serde(with = "dec_str")]
    pub balance: Dec,
}

pub fn trial_balance(book: &Book, at: BusinessTime, require_final: bool) -> Result<TrialBalance> {
    let entries = scope(book, End::AtOrBefore(at), require_final)?;
    let idx = Index::new(book);
    let mut cols: HashMap<Uuid, (Dec, Dec)> = HashMap::new();
    for e in entries.iter().filter(|e| e.time <= at) {
        for leg in &e.legs {
            let c = cols.entry(leg.account_id).or_insert_with(|| (zero(), zero()));
            match leg.side {
                LegSide::Debit => c.0 += &leg.book_amount,
                LegSide::Credit => c.1 += &leg.book_amount,
            }
        }
    }
    let mut rows = Vec::new();
    let (mut total_debit, mut total_credit) = (zero(), zero());
    for id in idx.tree_order() {
        let (Some(a), Some((debit, credit))) = (idx.account(id), cols.get(&id)) else {
            continue;
        };
        let balance = if a.kind.debit_normal() { debit - credit } else { credit - debit };
        if is_zero(&balance) {
            continue;
        }
        total_debit += debit;
        total_credit += credit;
        rows.push(TrialBalanceRow {
            account_id: id,
            debit: debit.clone(),
            credit: credit.clone(),
            balance,
        });
    }
    Ok(TrialBalance {
        rows,
        total_debit,
        total_credit,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceSheet {
    pub assets: Vec<AccountNode>,
    pub liabilities: Vec<AccountNode>,
    pub equity: Vec<AccountNode>,
    #[serde(with = "dec_str")]
    pub unclosed_net_income: Dec,
    #[serde(with = "dec_str")]
    pub cash: Dec,
    #[serde(with = "dec_str")]
    pub total_assets: Dec,
    #[serde(with = "dec_str")]
    pub total_liabilities_and_equity: Dec,
}

pub fn balance_sheet(book: &Book, at: BusinessTime, require_final: bool) -> Result<BalanceSheet> {
    let entries = scope(book, End::AtOrBefore(at), require_final)?;
    let idx = Index::new(book);
    let sums = leaf_sums(&idx, entries, |e| e.time <= at, |_| true);
    let appear = nonzero_keys(&sums);
    let unclosed = total_of(&idx, AccountType::Income, &sums) - total_of(&idx, AccountType::Expense, &sums);
    let mut cash = zero();
    for (id, s) in &sums {
        if idx.account(*id).is_some_and(|a| a.is_cash_or_cash_equivalent) {
            cash += &s.natural;
        }
    }
    let total_assets = total_of(&idx, AccountType::Asset, &sums);
    let total_le = total_of(&idx, AccountType::Liability, &sums) + total_of(&idx, AccountType::Equity, &sums) + &unclosed;
    Ok(BalanceSheet {
        assets: nodes_of(&idx, AccountType::Asset, &sums, &appear),
        liabilities: nodes_of(&idx, AccountType::Liability, &sums, &appear),
        equity: nodes_of(&idx, AccountType::Equity, &sums, &appear),
        unclosed_net_income: unclosed,
        cash,
        total_assets,
        total_liabilities_and_equity: total_le,
    })
}

/// Leaves that appear in a period report: non-zero opening, closing or movement.
fn period_appear(opening: &HashMap<Uuid, Sums>, closing: &HashMap<Uuid, Sums>, movement: &HashMap<Uuid, Sums>) -> HashSet<Uuid> {
    let mut s = nonzero_keys(opening);
    s.extend(nonzero_keys(closing));
    s.extend(nonzero_keys(movement));
    s
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomeStatement {
    pub income: Vec<AccountNode>,
    pub expenses: Vec<AccountNode>,
    pub by_leg_type: Vec<LegTypeSubtotal>,
    #[serde(with = "dec_str")]
    pub net_income: Dec,
    pub taxable: TaxSplit,
    pub non_taxable: TaxSplit,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxSplit {
    #[serde(with = "dec_str")]
    pub income: Dec,
    #[serde(with = "dec_str")]
    pub expenses: Dec,
    #[serde(with = "dec_str")]
    pub net: Dec,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegTypeSubtotal {
    #[serde(rename = "type")]
    pub kind: LegType,
    #[serde(with = "dec_str")]
    pub income: Dec,
    #[serde(with = "dec_str")]
    pub expenses: Dec,
    #[serde(with = "dec_str")]
    pub net: Dec,
}

pub fn income_statement(book: &Book, range: BusinessTimeRange, require_final: bool) -> Result<IncomeStatement> {
    range_ok(&range)?;
    let entries = scope(book, End::Before(range.until), require_final)?;
    let idx = Index::new(book);
    let in_range = |e: &Entry| range.contains(e.time) && e.kind != EntryType::Closing;
    let movement = leaf_sums(&idx, entries.iter().copied(), in_range, |_| true);
    let opening = leaf_sums(&idx, entries.iter().copied(), |e| e.time < range.from, |_| true);
    let closing = leaf_sums(&idx, entries.iter().copied(), |e| e.time < range.until, |_| true);
    let appear = period_appear(&opening, &closing, &movement);

    let mut by_type: BTreeMap<LegType, (Dec, Dec)> = LegType::ALL.iter().map(|&t| (t, (zero(), zero()))).collect();
    let (mut tax, mut non_tax) = ((zero(), zero()), (zero(), zero()));
    for e in entries.iter().filter(|e| in_range(e)) {
        for leg in &e.legs {
            let Some(a) = idx.account(leg.account_id) else { continue };
            let n = leg.natural(a.kind);
            let slot = by_type
                .get_mut(&leg.metadata.kind.unwrap_or(LegType::Other))
                .expect("all types present");
            let split = if leg.is_tax_relevant { &mut tax } else { &mut non_tax };
            match a.kind {
                AccountType::Income => {
                    slot.0 += &n;
                    split.0 += &n;
                }
                AccountType::Expense => {
                    slot.1 += &n;
                    split.1 += &n;
                }
                _ => {}
            }
        }
    }
    let split = |(i, x): (Dec, Dec)| TaxSplit {
        net: &i - &x,
        income: i,
        expenses: x,
    };
    let taxable = split(tax);
    let non_taxable = split(non_tax);
    Ok(IncomeStatement {
        income: nodes_of(&idx, AccountType::Income, &movement, &appear),
        expenses: nodes_of(&idx, AccountType::Expense, &movement, &appear),
        by_leg_type: LegType::ALL
            .iter()
            .map(|t| {
                let (i, x) = by_type[t].clone();
                LegTypeSubtotal {
                    kind: *t,
                    net: &i - &x,
                    income: i,
                    expenses: x,
                }
            })
            .collect(),
        net_income: &taxable.net + &non_taxable.net,
        taxable,
        non_taxable,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CashFlowStatement {
    #[serde(with = "dec_str")]
    pub opening_cash: Dec,
    pub sections: Vec<CashFlowSection>,
    #[serde(with = "dec_str")]
    pub net_change: Dec,
    #[serde(with = "dec_str")]
    pub closing_cash: Dec,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CashFlowSection {
    #[serde(rename = "type")]
    pub kind: LegType,
    pub lines: Vec<CashFlowLine>,
    #[serde(with = "dec_str")]
    pub inflow: Dec,
    #[serde(with = "dec_str")]
    pub outflow: Dec,
    #[serde(with = "dec_str")]
    pub net: Dec,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CashFlowLine {
    pub account_id: Uuid,
    #[serde(with = "dec_str")]
    pub inflow: Dec,
    #[serde(with = "dec_str")]
    pub outflow: Dec,
    #[serde(with = "dec_str")]
    pub net: Dec,
}

pub fn cash_flow_statement(book: &Book, range: BusinessTimeRange, require_final: bool) -> Result<CashFlowStatement> {
    range_ok(&range)?;
    let entries = scope(book, End::Before(range.until), require_final)?;
    let idx = Index::new(book);
    let is_cash = |id: Uuid| idx.account(id).is_some_and(|a| a.is_cash_or_cash_equivalent);
    let mut opening_cash = zero();
    let mut flows: HashMap<(LegType, Uuid), (Dec, Dec)> = HashMap::new();
    for e in &entries {
        if e.time < range.from {
            for leg in e.legs.iter().filter(|l| is_cash(l.account_id)) {
                opening_cash += leg.natural(AccountType::Asset);
            }
        } else if range.contains(e.time) {
            for leg in e.legs.iter().filter(|l| !is_cash(l.account_id) && !is_zero(&l.cash_amount)) {
                let f = flows
                    .entry((leg.metadata.kind.unwrap_or(LegType::Other), leg.account_id))
                    .or_insert_with(|| (zero(), zero()));
                match leg.side {
                    LegSide::Credit => f.0 += &leg.cash_amount,
                    LegSide::Debit => f.1 += &leg.cash_amount,
                }
            }
        }
    }
    let order = idx.tree_order();
    let mut net_change = zero();
    let sections = LegType::SECTIONS
        .iter()
        .map(|&kind| {
            let lines: Vec<CashFlowLine> = order
                .iter()
                .filter_map(|id| flows.get(&(kind, *id)).map(|(i, o)| (id, i, o)))
                .filter(|(_, i, o)| !is_zero(i) || !is_zero(o))
                .map(|(id, i, o)| CashFlowLine {
                    account_id: *id,
                    net: i - o,
                    inflow: i.clone(),
                    outflow: o.clone(),
                })
                .collect();
            let (mut inflow, mut outflow) = (zero(), zero());
            for l in &lines {
                inflow += &l.inflow;
                outflow += &l.outflow;
            }
            let net = &inflow - &outflow;
            net_change += &net;
            CashFlowSection {
                kind,
                lines,
                inflow,
                outflow,
                net,
            }
        })
        .collect();
    Ok(CashFlowStatement {
        closing_cash: &opening_cash + &net_change,
        opening_cash,
        sections,
        net_change,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesInEquity {
    pub rows: Vec<EquityRow>,
    #[serde(with = "dec_str")]
    pub opening_unclosed_net_income: Dec,
    #[serde(with = "dec_str")]
    pub closing_unclosed_net_income: Dec,
    #[serde(with = "dec_str")]
    pub opening_total: Dec,
    #[serde(with = "dec_str")]
    pub closing_total: Dec,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquityRow {
    pub account_id: Uuid,
    #[serde(with = "dec_str")]
    pub opening: Dec,
    #[serde(with = "dec_str")]
    pub from_closing_entries: Dec,
    #[serde(with = "dec_str")]
    pub other_movements: Dec,
    #[serde(with = "dec_str")]
    pub closing: Dec,
}

pub fn changes_in_equity(book: &Book, range: BusinessTimeRange, require_final: bool) -> Result<ChangesInEquity> {
    range_ok(&range)?;
    let entries = scope(book, End::Before(range.until), require_final)?;
    let idx = Index::new(book);
    let opening = leaf_sums(&idx, entries.iter().copied(), |e| e.time < range.from, |_| true);
    let closing = leaf_sums(&idx, entries.iter().copied(), |e| e.time < range.until, |_| true);
    let by_closing = leaf_sums(
        &idx,
        entries.iter().copied(),
        |e| range.contains(e.time) && e.kind == EntryType::Closing,
        |_| true,
    );
    let other = leaf_sums(
        &idx,
        entries.iter().copied(),
        |e| range.contains(e.time) && e.kind != EntryType::Closing,
        |_| true,
    );
    let get = |m: &HashMap<Uuid, Sums>, id: &Uuid| m.get(id).map(|s| s.natural.clone()).unwrap_or_else(zero);
    let mut rows = Vec::new();
    let (mut opening_eq, mut closing_eq) = (zero(), zero());
    for id in idx.tree_order() {
        if !idx.account(id).is_some_and(|a| a.kind == AccountType::Equity) || !idx.is_leaf(id) {
            continue;
        }
        let row = EquityRow {
            account_id: id,
            opening: get(&opening, &id),
            from_closing_entries: get(&by_closing, &id),
            other_movements: get(&other, &id),
            closing: get(&closing, &id),
        };
        opening_eq += &row.opening;
        closing_eq += &row.closing;
        if [&row.opening, &row.from_closing_entries, &row.other_movements, &row.closing]
            .iter()
            .any(|d| !is_zero(d))
        {
            rows.push(row);
        }
    }
    let unclosed = |m: &HashMap<Uuid, Sums>| total_of(&idx, AccountType::Income, m) - total_of(&idx, AccountType::Expense, m);
    let opening_unclosed = unclosed(&opening);
    let closing_unclosed = unclosed(&closing);
    Ok(ChangesInEquity {
        rows,
        opening_total: &opening_eq + &opening_unclosed,
        closing_total: &closing_eq + &closing_unclosed,
        opening_unclosed_net_income: opening_unclosed,
        closing_unclosed_net_income: closing_unclosed,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientBalance {
    pub client_id: Uuid,
    pub accounts: Vec<AccountNode>,
    #[serde(with = "dec_str")]
    pub total: Dec,
}

pub fn client_balances(book: &Book, at: BusinessTime, require_final: bool, account_id: Option<Uuid>) -> Result<Vec<ClientBalance>> {
    let entries = scope(book, End::AtOrBefore(at), require_final)?;
    let idx = Index::new(book);
    let roots: Vec<Uuid> = match account_id {
        Some(id) if idx.account(id).is_none() => {
            return Err(Error::NotFound {
                what: "account".into(),
                id: id.to_string(),
            });
        }
        Some(id) => vec![id],
        None => idx.roots().to_vec(),
    };
    let mut out = Vec::new();
    for c in &book.clients {
        let sums = leaf_sums(
            &idx,
            entries.iter().copied(),
            |e| e.time <= at,
            |l| l.client_id == Some(c.unique_id),
        );
        let appear = nonzero_keys(&sums);
        if appear.is_empty() {
            continue;
        }
        let mut seen = HashSet::new();
        let accounts: Vec<AccountNode> = roots
            .iter()
            .filter_map(|&r| {
                let (n, shown) = node(&idx, r, &sums, &appear, &mut seen);
                shown.then_some(n)
            })
            .collect();
        if accounts.is_empty() {
            continue;
        }
        let mut total = zero();
        for n in &accounts {
            total += &n.balance;
        }
        out.push(ClientBalance {
            client_id: c.unique_id,
            accounts,
            total,
        });
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralLedger {
    pub account_id: Uuid,
    #[serde(with = "dec_str")]
    pub opening: Dec,
    pub lines: Vec<LedgerLine>,
    #[serde(with = "dec_str")]
    pub closing: Dec,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerLine {
    pub entry_id: Uuid,
    pub time: BusinessTime,
    pub state: EntryState,
    pub leg_account_id: Uuid,
    pub side: LegSide,
    #[serde(with = "dec_str")]
    pub book_amount: Dec,
    #[serde(with = "dec_str")]
    pub original_amount: Dec,
    pub original_currency: String,
    pub client_id: Option<Uuid>,
    #[serde(with = "dec_str")]
    pub running_balance: Dec,
}

pub fn general_ledger(book: &Book, account_id: Uuid, range: BusinessTimeRange, require_final: bool) -> Result<GeneralLedger> {
    range_ok(&range)?;
    let entries = scope(book, End::Before(range.until), require_final)?;
    let idx = Index::new(book);
    let Some(account) = idx.account(account_id) else {
        return Err(Error::NotFound {
            what: "account".into(),
            id: account_id.to_string(),
        });
    };
    let subtree: HashSet<Uuid> = idx.subtree(account_id).into_iter().collect();
    let mut opening = zero();
    for e in entries.iter().filter(|e| e.time < range.from) {
        for leg in e.legs.iter().filter(|l| subtree.contains(&l.account_id)) {
            opening += leg.natural(account.kind);
        }
    }
    let mut running = opening.clone();
    let mut lines = Vec::new();
    for e in entries.iter().filter(|e| range.contains(e.time)) {
        for leg in e.legs.iter().filter(|l| subtree.contains(&l.account_id)) {
            running += leg.natural(account.kind);
            lines.push(LedgerLine {
                entry_id: e.unique_id,
                time: e.time,
                state: e.state,
                leg_account_id: leg.account_id,
                side: leg.side,
                book_amount: leg.book_amount.clone(),
                original_amount: leg.original_amount.clone(),
                original_currency: leg.original_currency.clone(),
                client_id: leg.client_id,
                running_balance: running.clone(),
            });
        }
    }
    Ok(GeneralLedger {
        account_id,
        opening,
        lines,
        closing: running,
    })
}

pub fn journal(book: &Book, range: BusinessTimeRange, require_final: bool) -> Result<Vec<Entry>> {
    range_ok(&range)?;
    let entries = scope(book, End::Before(range.until), require_final)?;
    Ok(entries.into_iter().filter(|e| range.contains(e.time)).cloned().collect())
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegTagTotal {
    pub tag_id: Uuid,
    #[serde(with = "dec_str")]
    pub debit: Dec,
    #[serde(with = "dec_str")]
    pub credit: Dec,
}

pub fn leg_tag_summary(book: &Book, range: BusinessTimeRange, require_final: bool) -> Result<Vec<LegTagTotal>> {
    range_ok(&range)?;
    let entries = scope(book, End::Before(range.until), require_final)?;
    let mut totals: HashMap<Uuid, (String, Dec, Dec)> = HashMap::new();
    for e in entries.iter().filter(|e| range.contains(e.time)) {
        for leg in &e.legs {
            let mut seen = HashSet::new();
            for tag in leg.metadata.tags.iter().filter(|t| seen.insert(t.unique_id)) {
                let t = totals
                    .entry(tag.unique_id)
                    .or_insert_with(|| (tag.name_slug.clone(), zero(), zero()));
                match leg.side {
                    LegSide::Debit => t.1 += &leg.book_amount,
                    LegSide::Credit => t.2 += &leg.book_amount,
                }
            }
        }
    }
    let mut out: Vec<(String, LegTagTotal)> = totals
        .into_iter()
        .map(|(id, (slug, debit, credit))| (slug, LegTagTotal { tag_id: id, debit, credit }))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.tag_id.cmp(&b.1.tag_id)));
    Ok(out.into_iter().map(|(_, t)| t).collect())
}

/// The fiscal year that ends with closingMonth of `year`.
pub fn fiscal_year(book: &Book, year: i32) -> Result<BusinessTimeRange> {
    let m = book.closing_month;
    if !(1..=12).contains(&m) {
        return Err(Error::BadRequest {
            message: "closingMonth is within [1, 12]".into(),
        });
    }
    let first_month = m % 12 + 1;
    let start_year = if m == 12 { year } else { year - 1 };
    let date = |y: i32| {
        u16::try_from(y)
            .ok()
            .and_then(|y| Date::new(y, first_month, 1))
            .ok_or_else(|| Error::BadRequest {
                message: format!("year {year} is outside 0000-9999"),
            })
    };
    Ok(BusinessTimeRange {
        from: BusinessTime::start_of(date(start_year)?),
        until: BusinessTime::start_of(date(start_year + 1)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::Change;
    use crate::book::tests::*;

    fn range(from: &str, until: &str) -> BusinessTimeRange {
        BusinessTimeRange {
            from: t(from),
            until: t(until),
        }
    }

    fn sample() -> Fixture {
        let mut f = fixture();
        let entries = vec![
            entry(
                "2026-01-01T09:00",
                vec![
                    leg(&f.bank, LegSide::Debit, "1000", "1000", None),
                    leg(&f.capital, LegSide::Credit, "1000", "1000", Some(LegType::Financing)),
                ],
            ),
            entry(
                "2026-01-02T10:00",
                vec![
                    leg(&f.bank, LegSide::Debit, "150", "150", None),
                    leg(&f.loan, LegSide::Credit, "100", "100", Some(LegType::Financing)),
                    leg(&f.revenue, LegSide::Credit, "50", "50", Some(LegType::Operating)),
                ],
            ),
            entry(
                "2026-02-01T10:00",
                vec![
                    leg(&f.supplies, LegSide::Debit, "30", "30", Some(LegType::Operating)),
                    leg(&f.bank, LegSide::Credit, "30", "30", None),
                ],
            ),
        ];
        for e in entries {
            f.book.create_entry(e).unwrap();
        }
        f
    }

    #[test]
    fn balance_sheet_balances() {
        let f = sample();
        let bs = balance_sheet(&f.book, t("2026-12-31T23:59"), true).unwrap();
        assert_eq!(bs.total_assets, d("1120"));
        assert_eq!(bs.total_liabilities_and_equity, d("1120"));
        assert_eq!(bs.unclosed_net_income, d("20"));
        assert_eq!(bs.cash, d("1120"));
        let tb = trial_balance(&f.book, t("2026-12-31T23:59"), true).unwrap();
        assert_eq!(tb.total_debit, tb.total_credit);
        assert_eq!(tb.rows.len(), 5);
    }

    #[test]
    fn income_statement_splits_tax() {
        let mut f = sample();
        let mut e = entry(
            "2026-03-01T10:00",
            vec![
                leg(&f.supplies, LegSide::Debit, "5", "5", Some(LegType::Operating)),
                leg(&f.bank, LegSide::Credit, "5", "5", None),
            ],
        );
        e.legs[0].is_tax_relevant = false;
        f.book.create_entry(e).unwrap();
        let is = income_statement(&f.book, range("2026-01-01T00:00", "2027-01-01T00:00"), true).unwrap();
        assert_eq!(is.net_income, d("15"));
        assert_eq!(is.taxable.net, d("20"));
        assert_eq!(is.non_taxable.net, d("-5"));
    }

    #[test]
    fn cash_flow_reconciles() {
        let f = sample();
        let cf = cash_flow_statement(&f.book, range("2026-01-02T00:00", "2027-01-01T00:00"), true).unwrap();
        assert_eq!(cf.opening_cash, d("1000"));
        assert_eq!(cf.closing_cash, d("1120"));
        let net: BTreeMap<LegType, Dec> = cf.sections.iter().map(|s| (s.kind, s.net.clone())).collect();
        assert_eq!(net[&LegType::Financing], d("100"));
        assert_eq!(net[&LegType::Operating], d("20"));
    }

    #[test]
    fn require_final_covers_history() {
        let mut f = sample();
        let id = f.book.entries[0].unique_id;
        let change = Change {
            author: "t".into(),
            reason: "recheck".into(),
            recorded_at: None,
        };
        f.book.set_entry_state(id, EntryState::Draft, change).unwrap();
        let r = income_statement(&f.book, range("2026-02-01T00:00", "2026-03-01T00:00"), true);
        assert_eq!(r, Err(Error::DraftInScope { entry_ids: vec![id] }));
        assert!(income_statement(&f.book, range("2026-02-01T00:00", "2026-03-01T00:00"), false).is_ok());
    }

    #[test]
    fn ledger_and_equity() {
        let f = sample();
        let gl = general_ledger(&f.book, f.bank.unique_id, range("2026-01-02T00:00", "2027-01-01T00:00"), true).unwrap();
        assert_eq!(gl.opening, d("1000"));
        assert_eq!(gl.closing, d("1120"));
        assert_eq!(gl.lines.len(), 2);
        let eq = changes_in_equity(&f.book, range("2026-01-01T00:00", "2027-01-01T00:00"), true).unwrap();
        assert_eq!(eq.closing_total, d("1020"));
    }

    #[test]
    fn fiscal_years() {
        let mut f = fixture();
        assert_eq!(fiscal_year(&f.book, 2026).unwrap().from.to_string(), "2026-01-01T-2147483648:00");
        f.book.closing_month = 3;
        let r = fiscal_year(&f.book, 2026).unwrap();
        assert_eq!(r.from.date.to_string(), "2025-04-01");
        assert_eq!(r.until.date.to_string(), "2026-04-01");
    }
}
