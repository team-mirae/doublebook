//! Terraform accountant (Go engine, `acct_*` tables) to doublebook.
//!
//! Input is one JSON array per table, rows as the database returns them
//! (`scripts/export-terraform.sh` writes them). The conversion is
//! deterministic: the same export always yields the same books and ids.
//! See `docs/MIGRATION.md` for every mapping decision.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::book::{Problem, validate_book};
use crate::model::*;
use crate::time::{BusinessTime, Date, rfc3339_from_unix_millis};

/// Prefix of every `extra` key the migration writes, so nothing collides with
/// metadata carried over verbatim.
const P: &str = "terraform.";

const NAMESPACE: Uuid = Uuid::from_u128(0x0b1d_0e5c_7a1f_4c3e_9d2a_5f6b_8e7c_1a2b);

// ---- input -------------------------------------------------------------

fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Bool(b) => b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        Value::String(s) => matches!(s.as_str(), "1" | "true" | "t" | "TRUE"),
        _ => false,
    })
}

fn lenient_i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    })
}

/// null and "" both mean absent.
fn opt_str<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) if !s.is_empty() => Some(s),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(_) | Value::Array(_) => None,
        _ => None,
    })
}

/// A JSON column that may arrive as text or as already-parsed JSON.
fn json_text<'de, D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
        Value::Null => Value::Object(Map::new()),
        v => v,
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Export {
    /// True for a magic-company SQLite export, whose time columns hold the
    /// +24-biased sort key (`2026-01-01T33:30:04.000`) instead of the wire form.
    #[serde(default)]
    pub sort_keys: bool,
    #[serde(default)]
    pub books: Vec<TBook>,
    #[serde(default)]
    pub currencies: Vec<TCurrency>,
    #[serde(default)]
    pub accounts: Vec<TAccount>,
    #[serde(default)]
    pub clients: Vec<TClient>,
    #[serde(default)]
    pub batches: Vec<TBatch>,
    #[serde(default)]
    pub entries: Vec<TEntry>,
    #[serde(default)]
    pub postings: Vec<TPosting>,
    #[serde(default)]
    pub entry_revisions: Vec<TRevision>,
    #[serde(default)]
    pub cash_flow_pairs: Vec<TPair>,
    #[serde(default)]
    pub statement_notes: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TBook {
    pub id: String,
    pub name: String,
    pub base_currency: String,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub closing_month: i64,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub sort_order: i64,
    #[serde(default, deserialize_with = "opt_str")]
    pub org_id: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub created_at: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TCurrency {
    pub book_id: String,
    pub code: String,
    pub name: String,
    #[serde(default, deserialize_with = "opt_str")]
    pub symbol: Option<String>,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub display_decimals: i64,
    #[serde(default, deserialize_with = "lenient_bool")]
    pub retired: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TAccount {
    pub book_id: String,
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, deserialize_with = "opt_str")]
    pub parent_id: Option<String>,
    pub name: String,
    #[serde(default, deserialize_with = "opt_str")]
    pub short_name: Option<String>,
    #[serde(default, deserialize_with = "json_text")]
    pub name_i18n: Value,
    #[serde(default, deserialize_with = "opt_str")]
    pub contra_of: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub path: Option<String>,
    pub currency: String,
    #[serde(default, deserialize_with = "opt_str")]
    pub classification: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub asset_category: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub liability_category: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub expiry_date: Option<String>,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub sort_order: i64,
    #[serde(default, deserialize_with = "lenient_bool")]
    pub retired: bool,
    #[serde(default, deserialize_with = "opt_str")]
    pub created_at: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TClient {
    pub id: String,
    pub book_id: String,
    pub name: String,
    #[serde(default, deserialize_with = "opt_str")]
    pub note: Option<String>,
    #[serde(default, deserialize_with = "lenient_bool")]
    pub retired: bool,
    #[serde(default, deserialize_with = "opt_str")]
    pub created_at: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TBatch {
    pub id: String,
    pub book_id: String,
    pub label: String,
    #[serde(default, deserialize_with = "opt_str")]
    pub note: Option<String>,
    pub kind: String,
    #[serde(default, deserialize_with = "opt_str")]
    pub generator: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TEntry {
    pub id: String,
    #[serde(default)]
    pub book_id: String,
    pub ts: String,
    #[serde(default, deserialize_with = "lenient_i64", alias = "intradaySeq")]
    pub intraday_seq: i64,
    #[serde(default, deserialize_with = "opt_str")]
    pub payee: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub detail: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub memo: Option<String>,
    #[serde(default, deserialize_with = "opt_str", alias = "clientId")]
    pub client_id: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub origin: Option<String>,
    #[serde(default, deserialize_with = "opt_str", alias = "batchId")]
    pub batch_id: Option<String>,
    pub status: String,
    #[serde(default, deserialize_with = "opt_str", alias = "priorStatus")]
    pub prior_status: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub created_at: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub updated_at: Option<String>,
    /// Only in revision snapshots.
    #[serde(default)]
    pub postings: Vec<TPosting>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TPosting {
    pub id: String,
    #[serde(default, alias = "entryId")]
    pub entry_id: String,
    #[serde(alias = "accountId")]
    pub account_id: String,
    pub side: String,
    pub amount: String,
    #[serde(alias = "baseAmount")]
    pub base_amount: String,
    pub currency: String,
    #[serde(default, deserialize_with = "json_text")]
    pub metadata: Value,
    #[serde(default, deserialize_with = "lenient_i64", alias = "sortOrder")]
    pub sort_order: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TRevision {
    pub id: String,
    pub entry_id: String,
    #[serde(deserialize_with = "lenient_i64")]
    pub revision: i64,
    pub actor: String,
    #[serde(default, deserialize_with = "opt_str")]
    pub reason: Option<String>,
    #[serde(deserialize_with = "json_text")]
    pub snapshot: Value,
    #[serde(default, deserialize_with = "opt_str")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TPair {
    pub id: String,
    pub book_id: String,
    #[serde(default, deserialize_with = "opt_str")]
    pub cash_account_id: Option<String>,
    pub counter_account_id: String,
    pub direction: String,
    #[serde(default, deserialize_with = "opt_str")]
    pub classification: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub start_date: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub end_date: Option<String>,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub sort_order: i64,
}

// ---- output ------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookResult {
    pub book: Book,
    /// Data the target model cannot hold exactly. Nothing is silently lost:
    /// anything without a field lands in `extra`.
    pub warnings: Vec<Warning>,
    /// validate_book on the result. Empty means the book is valid.
    pub problems: Vec<Problem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Migration {
    pub books: Vec<BookResult>,
}

// ---- ids ---------------------------------------------------------------

/// A uuidv4-shaped id derived from a name, so reruns give the same ids.
pub fn det_v4(kind: &str, key: &str) -> Uuid {
    let mut b = *Uuid::new_v5(&NAMESPACE, format!("{kind}:{key}").as_bytes()).as_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    Uuid::from_bytes(b)
}

/// Decodes a Crockford ULID into (milliseconds, 80 random bits).
pub fn decode_ulid(s: &str) -> Option<(u64, u128)> {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    if s.len() != 26 {
        return None;
    }
    let mut v: u128 = 0;
    for (i, c) in s.bytes().enumerate() {
        let d = ALPHABET.iter().position(|&a| a == c.to_ascii_uppercase())? as u128;
        if i == 0 && d > 7 {
            return None;
        }
        v = (v << 5) | d;
    }
    Some(((v >> 80) as u64, v & ((1u128 << 80) - 1)))
}

/// uuidv7 from a millisecond timestamp and 80 random bits.
pub fn v7(ms: u64, rand: u128) -> Uuid {
    let ms = (ms & 0xffff_ffff_ffff) as u128;
    let rand_a = (rand >> 68) & 0xfff;
    let rand_b = rand & ((1u128 << 62) - 1);
    Uuid::from_u128((ms << 80) | (0x7 << 76) | (rand_a << 64) | (0b10 << 62) | rand_b)
}

// ---- time --------------------------------------------------------------

/// A Terraform AccountingDateTime: `YYYY-MM-DDTHH:MM:SS.mmm`, the sign of a
/// negative hour applying to the whole time of day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Adt {
    pub date: Date,
    pub millis: i64,
}

pub fn parse_adt(s: &str) -> Option<Adt> {
    let s = s.trim();
    let (date, time) = s.split_once('T')?;
    let date = Date::parse(date)?;
    let (negative, time) = match time.strip_prefix('-') {
        Some(t) => (true, t),
        None => (false, time),
    };
    let b = time.as_bytes();
    if b.len() != 12 || b[2] != b':' || b[5] != b':' || b[8] != b'.' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = &time[r];
        part.bytes().all(|c| c.is_ascii_digit()).then(|| part.parse().ok())?
    };
    let (h, m, sec, ms) = (n(0..2)?, n(3..5)?, n(6..8)?, n(9..12)?);
    if m > 59 || sec > 59 {
        return None;
    }
    let magnitude = h * 3_600_000 + m * 60_000 + sec * 1000 + ms;
    Some(Adt {
        date,
        millis: if negative { -magnitude } else { magnitude },
    })
}

/// Reads the +24-biased sort-key form (hour 00-72, no sign).
pub fn parse_sort_key(s: &str) -> Option<Adt> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() != 23 || b[10] != b'T' || b[11] == b'-' {
        return None;
    }
    let wire = parse_adt(s)?;
    Some(Adt {
        date: wire.date,
        millis: wire.millis - 24 * 3_600_000,
    })
}

impl Adt {
    /// The Terraform wire form.
    pub fn to_wire(self) -> String {
        let sign = if self.millis < 0 { "-" } else { "" };
        let m = self.millis.unsigned_abs();
        format!(
            "{}T{sign}{:02}:{:02}:{:02}.{:03}",
            self.date,
            m / 3_600_000,
            m / 60_000 % 60,
            m / 1000 % 60,
            m % 1000
        )
    }

    /// Floors to the minute, which keeps the order of distinct times.
    pub fn to_business_time(self) -> Option<BusinessTime> {
        BusinessTime::from_minutes(self.date, self.millis.div_euclid(60_000))
    }

    pub fn has_sub_minute(self) -> bool {
        self.millis.rem_euclid(60_000) != 0
    }

    /// Terraform stamps wall-clock times in KST (UTC+9).
    pub fn kst_to_unix_millis(self) -> i64 {
        self.date.days_since_epoch() * 86_400_000 + self.millis - 9 * 3_600_000
    }
}

// ---- chart helpers -----------------------------------------------------

struct Chart<'a> {
    by_id: HashMap<&'a str, &'a TAccount>,
}

impl<'a> Chart<'a> {
    fn get(&self, id: &str) -> Option<&'a TAccount> {
        self.by_id.get(id).copied()
    }

    /// Ancestor chain including self, nearest first; cycle-guarded.
    fn chain(&self, id: &str) -> Vec<&'a TAccount> {
        let mut out: Vec<&TAccount> = Vec::new();
        let mut cur = self.get(id);
        while let Some(a) = cur {
            if out.iter().any(|x| x.id == a.id) {
                break;
            }
            out.push(a);
            cur = a.parent_id.as_deref().and_then(|p| self.get(p));
        }
        out
    }

    fn resolve(&self, id: &str, field: impl Fn(&TAccount) -> Option<&String>) -> Option<String> {
        self.chain(id).into_iter().find_map(|a| field(a).cloned())
    }

    fn classification(&self, id: &str) -> Option<String> {
        self.resolve(id, |a| a.classification.as_ref())
    }

    fn asset_category(&self, id: &str) -> Option<String> {
        self.resolve(id, |a| a.asset_category.as_ref())
    }

    fn liability_category(&self, id: &str) -> Option<String> {
        self.resolve(id, |a| a.liability_category.as_ref())
    }

    fn contra_target(&self, a: &'a TAccount) -> &'a TAccount {
        a.contra_of.as_deref().and_then(|c| self.get(c)).unwrap_or(a)
    }

    /// CashFlow.isCashAccount.
    fn is_cash(&self, a: &TAccount) -> bool {
        if a.kind != "asset" {
            return false;
        }
        let target = self.contra_target(a);
        matches!(self.asset_category(&target.id).as_deref(), Some("cash" | "cash_equivalent"))
    }

    /// CashFlow.cashFlowBucket: the section of a non-cash counterpart absent a pair override.
    fn bucket(&self, a: &'a TAccount) -> String {
        match a.kind.as_str() {
            "equity" => self.classification(&a.id).unwrap_or_else(|| "financing".into()),
            "income" | "expense" => self.classification(&a.id).unwrap_or_else(|| "operating".into()),
            "asset" => {
                let target = self.contra_target(a);
                if let Some(c) = self.classification(&target.id) {
                    return c;
                }
                match self.asset_category(&target.id).as_deref() {
                    Some("investment" | "operating_asset") => "investing".into(),
                    Some("other") => "other".into(),
                    _ => "operating".into(),
                }
            }
            _ => {
                if let Some(c) = self.classification(&a.id) {
                    return c;
                }
                match self.liability_category(&a.id).as_deref() {
                    Some("line_of_credit" | "amortizing_loan") => "financing".into(),
                    Some("other") => "other".into(),
                    _ => "operating".into(),
                }
            }
        }
    }
}

fn leg_type(section: &str) -> LegType {
    match section {
        "investing" => LegType::Investing,
        "financing" => LegType::Financing,
        "fx" => LegType::FxEffects,
        "other" | "unclassified" => LegType::Other,
        _ => LegType::Operating, // operating, and tax (no tax section in doublebook)
    }
}

const SECTION_ORDER: [&str; 7] = ["operating", "investing", "financing", "tax", "fx", "other", "unclassified"];

// ---- cash flow pair rules (classification only) -------------------------

struct Rules<'a> {
    exact: HashMap<(String, String, String), Vec<&'a TPair>>,
    any_cash: HashMap<(String, String), Vec<&'a TPair>>,
}

impl<'a> Rules<'a> {
    fn new(chart: &Chart, pairs: &'a [TPair], book_id: &str) -> Rules<'a> {
        let mut list: Vec<&TPair> = pairs
            .iter()
            .filter(|p| p.book_id == book_id && chart.get(&p.counter_account_id).is_some())
            .collect();
        // The order the store loads them in.
        list.sort_by(|a, b| {
            (
                a.sort_order,
                &a.counter_account_id,
                &a.direction,
                a.start_date.as_deref().unwrap_or("0000-01-01"),
                &a.id,
            )
                .cmp(&(
                    b.sort_order,
                    &b.counter_account_id,
                    &b.direction,
                    b.start_date.as_deref().unwrap_or("0000-01-01"),
                    &b.id,
                ))
        });
        let mut exact: HashMap<(String, String, String), Vec<&TPair>> = HashMap::new();
        let mut any_cash: HashMap<(String, String), Vec<&TPair>> = HashMap::new();
        for p in list {
            match &p.cash_account_id {
                Some(c) => exact
                    .entry((c.clone(), p.counter_account_id.clone(), p.direction.clone()))
                    .or_default()
                    .push(p),
                None => any_cash
                    .entry((p.counter_account_id.clone(), p.direction.clone()))
                    .or_default()
                    .push(p),
            }
        }
        let newest_first = |l: &mut Vec<&TPair>| {
            l.sort_by(|a, b| {
                let (sa, sb) = (
                    a.start_date.as_deref().unwrap_or("0000-01-01"),
                    b.start_date.as_deref().unwrap_or("0000-01-01"),
                );
                sb.cmp(sa).then(a.sort_order.cmp(&b.sort_order))
            })
        };
        exact.values_mut().for_each(newest_first);
        any_cash.values_mut().for_each(newest_first);
        Rules { exact, any_cash }
    }

    fn valid_at<'b>(list: Option<&'b Vec<&'a TPair>>, date: &str) -> Option<&'a TPair> {
        list?
            .iter()
            .copied()
            .find(|p| p.start_date.as_deref().unwrap_or("0000-01-01") <= date && date <= p.end_date.as_deref().unwrap_or("9999-12-31"))
    }

    /// CashFlow.ruleFor.
    fn rule_for(&self, chart: &Chart, cash: &str, counter: &str, direction: &str, date: &str) -> Option<&'a TPair> {
        if self.exact.is_empty() && self.any_cash.is_empty() {
            return None;
        }
        for c in chart.chain(counter) {
            if !self.exact.is_empty() {
                for k in chart.chain(cash) {
                    let key = (k.id.clone(), c.id.clone(), direction.to_string());
                    if let Some(hit) = Self::valid_at(self.exact.get(&key), date) {
                        return Some(hit);
                    }
                }
            }
            if let Some(hit) = Self::valid_at(self.any_cash.get(&(c.id.clone(), direction.to_string())), date) {
                return Some(hit);
            }
        }
        None
    }
}

// ---- cash allocation (port of CashFlow's per-entry fill) ---------------

struct MLeg {
    account: String,
    reval: bool,
    remaining: Dec,
    /// Indexes of the postings merged into this leg.
    postings: Vec<usize>,
    /// Amount allocated per section, for counter legs.
    sections: BTreeMap<usize, Dec>,
    allocated: Dec,
}

fn zero() -> Dec {
    Dec::from(0)
}

fn positive(d: &Dec) -> bool {
    *d > zero()
}

struct Allocation {
    /// Cash amount per posting index.
    cash: Vec<Dec>,
    /// Section per posting index, when it received cash.
    section: Vec<Option<String>>,
    mixed: Vec<usize>,
}

struct Leg<'p> {
    p: &'p TPosting,
    side: LegSide,
    base: Dec,
    amount: Dec,
}

fn allocate(chart: &Chart, rules: &Rules, legs: &[Leg], date: &str) -> Allocation {
    let n = legs.len();
    let mut out = Allocation {
        cash: vec![zero(); n],
        section: vec![None; n],
        mixed: vec![],
    };
    let is_cash = |i: usize| chart.get(&legs[i].p.account_id).is_some_and(|a| chart.is_cash(a));
    if !(0..n).any(is_cash) {
        return out;
    }
    let order = |a: &MLeg, b: &MLeg| {
        let so = |id: &str| chart.get(id).map(|x| x.sort_order).unwrap_or(0);
        b.remaining
            .cmp(&a.remaining)
            .then(so(&a.account).cmp(&so(&b.account)))
            .then(a.account.cmp(&b.account))
            .then(a.reval.cmp(&b.reval))
    };
    // cash in, cash out, non-cash debit, non-cash credit
    let mut groups: [Vec<MLeg>; 4] = Default::default();
    for (i, l) in legs.iter().enumerate() {
        if l.base == zero() {
            continue;
        }
        let g = match (is_cash(i), l.side) {
            (true, LegSide::Debit) => 0,
            (true, LegSide::Credit) => 1,
            (false, LegSide::Debit) => 2,
            (false, LegSide::Credit) => 3,
        };
        let reval = l.amount == zero() && l.base != zero();
        match groups[g].iter_mut().find(|m| m.account == l.p.account_id && m.reval == reval) {
            Some(m) => {
                m.remaining += &l.base;
                m.postings.push(i);
            }
            None => groups[g].push(MLeg {
                account: l.p.account_id.clone(),
                reval,
                remaining: l.base.clone(),
                postings: vec![i],
                sections: BTreeMap::new(),
                allocated: zero(),
            }),
        }
    }
    for g in &mut groups {
        g.sort_by(order);
    }
    let [cash_in, cash_out, nc_debit, nc_credit] = &mut groups;
    for (cash, counter, direction) in [(cash_in, nc_credit, "in"), (cash_out, nc_debit, "out")] {
        fill(chart, rules, cash, counter, direction, date);
        // Every allocated amount goes back onto the merged postings in order.
        for m in counter.iter() {
            let mut left = m.allocated.clone();
            for &i in &m.postings {
                let take = if left > legs[i].base { legs[i].base.clone() } else { left.clone() };
                out.cash[i] = take.clone();
                left -= take;
            }
            if let Some((&best, _)) = m.sections.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))) {
                for &i in &m.postings {
                    out.section[i] = Some(SECTION_ORDER[best].to_string());
                }
                if m.sections.len() > 1 {
                    out.mixed.extend(m.postings.iter().copied());
                }
            }
        }
        for m in cash.iter() {
            for &i in &m.postings {
                out.cash[i] = legs[i].base.clone();
            }
        }
    }
    // Cash legs are always fully cash; zero-base legs above stay 0.
    for i in (0..n).filter(|&i| is_cash(i)) {
        out.cash[i] = legs[i].base.clone();
    }
    out
}

fn fill(chart: &Chart, rules: &Rules, cash: &mut [MLeg], counter: &mut [MLeg], direction: &str, date: &str) {
    let record = |c: &MLeg, k: &mut MLeg, amount: &Dec| {
        let section = if c.reval {
            "fx".to_string()
        } else {
            match rules
                .rule_for(chart, &c.account, &k.account, direction, date)
                .and_then(|r| r.classification.clone())
            {
                Some(cls) => cls,
                None => chart.get(&k.account).map(|a| chart.bucket(a)).unwrap_or_else(|| "other".into()),
            }
        };
        let s = SECTION_ORDER.iter().position(|x| *x == section).unwrap_or(5);
        *k.sections.entry(s).or_insert_with(zero) += amount;
        k.allocated += amount;
    };
    // pass one: each restatement takes an exactly equal counterpart
    for c in cash.iter_mut() {
        if !c.reval || !positive(&c.remaining) {
            continue;
        }
        let mut found: Option<usize> = None;
        for (j, k) in counter.iter().enumerate() {
            if !positive(&k.remaining) || k.remaining != c.remaining {
                continue;
            }
            if found.is_none_or(|f| k.reval && !counter[f].reval) {
                found = Some(j);
            }
        }
        if let Some(j) = found {
            let amount = c.remaining.clone();
            record(c, &mut counter[j], &amount);
            c.remaining -= &amount;
            counter[j].remaining -= &amount;
        }
    }
    // pass two: unclaimed restatements; pass three: real movements
    for reval in [true, false] {
        let cash_idx: Vec<usize> = (0..cash.len())
            .filter(|&i| cash[i].reval == reval && positive(&cash[i].remaining))
            .collect();
        let counter_idx: Vec<usize> = (0..counter.len()).filter(|&j| positive(&counter[j].remaining)).collect();
        let mut cursor = 0;
        for &i in &cash_idx {
            while positive(&cash[i].remaining) && cursor < counter_idx.len() {
                let j = counter_idx[cursor];
                let amount = if cash[i].remaining < counter[j].remaining {
                    cash[i].remaining.clone()
                } else {
                    counter[j].remaining.clone()
                };
                record(&cash[i], &mut counter[j], &amount);
                cash[i].remaining -= &amount;
                counter[j].remaining -= &amount;
                if counter[j].remaining == zero() {
                    cursor += 1;
                }
            }
        }
    }
}

// ---- conversion --------------------------------------------------------

fn tag(book: &str, slug: &str, en: &str, ko: &str) -> Tag {
    Tag {
        unique_id: det_v4("tag", &format!("{book}/{slug}")),
        name_english: en.into(),
        name_korean: ko.into(),
        name_slug: slug.into(),
    }
}

fn category_tag(book: &str, kind: &str, cat: &str) -> Tag {
    let (slug, en, ko) = match (kind, cat) {
        ("asset", "cash") => ("cash", "Cash", "현금"),
        ("asset", "cash_equivalent") => ("cash-equivalent", "Cash equivalent", "현금성자산"),
        ("asset", "receivable") => ("receivable", "Receivable", "채권"),
        ("asset", "prepaid_expense") => ("prepaid-expense", "Prepaid expense", "선급비용"),
        ("asset", "operating_asset") => ("operating-asset", "Operating asset", "영업자산"),
        ("asset", "investment") => ("investment", "Investment", "투자자산"),
        ("asset", _) => ("other-asset", "Other asset", "기타자산"),
        (_, "credit_card") => ("credit-card", "Credit card", "신용카드"),
        (_, "unpaid_expense") => ("unpaid-expense", "Unpaid expense", "미지급비용"),
        (_, "line_of_credit") => ("line-of-credit", "Line of credit", "한도대출"),
        (_, "amortizing_loan") => ("amortizing-loan", "Amortizing loan", "분할상환대출"),
        (_, "provision") => ("provision", "Provision", "충당부채"),
        (_, "guarantee_to_third_party") => ("guarantee-to-third-party", "Guarantee to third party", "제3자 보증"),
        _ => ("other-liability", "Other liability", "기타부채"),
    };
    tag(book, slug, en, ko)
}

fn account_type(s: &str) -> Option<AccountType> {
    Some(match s {
        "asset" => AccountType::Asset,
        "liability" => AccountType::Liability,
        "equity" => AccountType::Equity,
        "income" => AccountType::Income,
        "expense" => AccountType::Expense,
        _ => return None,
    })
}

fn put(extra: &mut Extra, key: &str, value: Option<impl ToString>) {
    if let Some(v) = value {
        extra.insert(format!("{P}{key}"), v.to_string());
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

struct Ctx<'a> {
    book_id: &'a str,
    chart: Chart<'a>,
    rules: Rules<'a>,
    accounts: HashMap<String, Uuid>,
    clients_by_id: HashMap<String, Uuid>,
    clients_by_name: HashMap<String, Uuid>,
    batches: HashMap<&'a str, &'a TBatch>,
    warnings: Vec<Warning>,
}

impl Ctx<'_> {
    fn warn(&mut self, code: &str, message: impl Into<String>, legacy_id: Option<&str>) {
        self.warnings.push(Warning {
            code: code.into(),
            message: message.into(),
            legacy_id: legacy_id.map(str::to_string),
        });
    }
}

/// Converts one Terraform entry (a row with its postings, or a revision
/// snapshot) into a doublebook entry with the given id.
fn convert_entry(ctx: &mut Ctx, e: &TEntry, postings: &[&TPosting], unique_id: Uuid, snapshot: bool) -> Option<Entry> {
    let Some(adt) = parse_adt(&e.ts) else {
        ctx.warn(
            "badTime",
            format!("entry time \"{}\" is not a Terraform AccountingDateTime; entry skipped", e.ts),
            Some(&e.id),
        );
        return None;
    };
    let time = adt.to_business_time()?;
    if adt.has_sub_minute() && !snapshot {
        ctx.warn(
            "secondsTruncated",
            format!("{} truncated to {time}; the original is in extra", e.ts),
            Some(&e.id),
        );
    }
    let mut posts: Vec<&TPosting> = postings.to_vec();
    posts.sort_by(|a, b| (a.sort_order, &a.id).cmp(&(b.sort_order, &b.id)));

    let mut legs_in: Vec<Leg> = Vec::new();
    for p in &posts {
        let parsed = (dec_str::parse(&p.amount), dec_str::parse(&p.base_amount));
        let (Ok(mut amount), Ok(mut base)) = parsed else {
            ctx.warn(
                "badAmount",
                format!("posting {} has an unreadable amount; entry skipped", p.id),
                Some(&e.id),
            );
            return None;
        };
        let mut side = if p.side == "credit" { LegSide::Credit } else { LegSide::Debit };
        if base < zero() || amount < zero() {
            if (base > zero() && amount < zero()) || (base < zero() && amount > zero()) {
                ctx.warn(
                    "signMismatch",
                    format!("posting {} has amount and baseAmount of opposite signs; magnitudes kept", p.id),
                    Some(&e.id),
                );
            } else {
                side = if side == LegSide::Debit { LegSide::Credit } else { LegSide::Debit };
            }
            amount = amount.abs();
            base = base.abs();
        }
        legs_in.push(Leg { p, side, base, amount });
    }
    let date = adt.date.to_string();
    let alloc = allocate(&ctx.chart, &ctx.rules, &legs_in, &date);
    if !alloc.mixed.is_empty() && !snapshot {
        ctx.warn(
            "mixedSection",
            "a leg moved cash for more than one cash flow section; the largest one became its LegType",
            Some(&e.id),
        );
    }

    let entry_client = e.client_id.as_ref().and_then(|c| ctx.clients_by_id.get(c).copied());
    if e.client_id.is_some() && entry_client.is_none() && !snapshot {
        ctx.warn(
            "unknownClient",
            format!("client {} does not exist", e.client_id.as_deref().unwrap_or("")),
            Some(&e.id),
        );
    }
    let mut legs = Vec::new();
    for (i, l) in legs_in.iter().enumerate() {
        let account_id = match ctx.accounts.get(&l.p.account_id) {
            Some(&id) => id,
            // A revision may name an account deleted since; history keeps a derived id.
            None if snapshot => det_v4("account", &format!("{}/{}", ctx.book_id, l.p.account_id)),
            None => {
                ctx.warn(
                    "unknownAccount",
                    format!("posting {} names unknown account {}; entry skipped", l.p.id, l.p.account_id),
                    Some(&e.id),
                );
                return None;
            }
        };
        let account = ctx.chart.get(&l.p.account_id);
        let mut extra = Extra::new();
        let mut client_id = entry_client;
        if let Value::Object(m) = &l.p.metadata {
            for (k, v) in m {
                extra.insert(k.clone(), value_text(v));
            }
            if let Some(c) = m.get("client").map(value_text) {
                match ctx.clients_by_id.get(&c).or_else(|| ctx.clients_by_name.get(&c)) {
                    Some(&id) => client_id = Some(id),
                    None if !snapshot => ctx.warn(
                        "unknownClient",
                        format!("posting {} names client \"{c}\", which does not exist; kept in extra", l.p.id),
                        Some(&e.id),
                    ),
                    None => {}
                }
            }
        }
        extra.insert(format!("{P}postingId"), l.p.id.clone());
        let cash = account.is_some_and(|a| ctx.chart.is_cash(a));
        let kind = if cash {
            None
        } else {
            let section = alloc.section[i]
                .clone()
                .or_else(|| account.map(|a| ctx.chart.bucket(a)))
                .unwrap_or_else(|| "other".into());
            Some(leg_type(&section))
        };
        let income_or_expense = account.is_some_and(|a| matches!(a.kind.as_str(), "income" | "expense"));
        legs.push(EntryLeg {
            account_id,
            side: l.side,
            client_id,
            book_amount: l.base.clone(),
            cash_amount: alloc.cash[i].clone(),
            original_amount: l.amount.clone(),
            original_currency: l.p.currency.clone(),
            is_tax_relevant: income_or_expense,
            metadata: LegMetadata { kind, tags: vec![] },
            extra,
        });
    }

    let batch = e.batch_id.as_deref().and_then(|b| ctx.batches.get(b).copied());
    let mut extra = Extra::new();
    put(&mut extra, "ts", Some(&e.ts));
    put(&mut extra, "intradaySeq", Some(e.intraday_seq));
    put(&mut extra, "origin", e.origin.as_ref());
    put(&mut extra, "status", Some(&e.status));
    put(&mut extra, "priorStatus", e.prior_status.as_ref());
    put(&mut extra, "clientId", e.client_id.as_ref());
    put(&mut extra, "batchId", e.batch_id.as_ref());
    if let Some(b) = batch {
        put(&mut extra, "batchKind", Some(&b.kind));
        put(&mut extra, "batchLabel", Some(&b.label));
        put(&mut extra, "batchNote", b.note.as_ref());
        put(&mut extra, "batchGenerator", b.generator.as_ref());
        put(&mut extra, "batchCreatedAt", b.created_at.as_ref());
    }
    put(&mut extra, "createdAt", e.created_at.as_ref());
    put(&mut extra, "updatedAt", e.updated_at.as_ref());
    let state = match e.status.as_str() {
        "draft" => EntryState::Draft,
        "void" => EntryState::Voided,
        _ => EntryState::Final,
    };
    Some(Entry {
        unique_id,
        legacy_id: Some(e.id.clone()),
        name: e.payee.clone().unwrap_or_default(),
        description: e.detail.clone().unwrap_or_default(),
        remarks: e.memo.clone().unwrap_or_default(),
        time,
        legs,
        kind: if batch.is_some_and(|b| b.kind == "closing") {
            EntryType::Closing
        } else {
            EntryType::Normal
        },
        state,
        revisions: vec![],
        extra,
    })
}

fn names(name: &str, i18n: &Value) -> (String, String) {
    let get = |k: &str| i18n.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    (
        get("en").unwrap_or_else(|| name.to_string()),
        get("ko").unwrap_or_else(|| name.to_string()),
    )
}

pub fn migrate(export: &Export) -> Migration {
    let converted;
    let export = if export.sort_keys {
        converted = keys_to_wire(export);
        &converted
    } else {
        export
    };
    let mut books = Vec::new();
    for tb in &export.books {
        books.push(migrate_book(export, tb));
    }
    Migration { books }
}

/// Rewrites every sort-key time column of a magic-company export to the wire
/// form, as Terraform's own import did. Revision snapshots already hold wire times.
fn keys_to_wire(x: &Export) -> Export {
    let w = |s: &mut String| {
        if let Some(a) = parse_sort_key(s) {
            *s = a.to_wire();
        }
    };
    let wo = |s: &mut Option<String>| {
        if let Some(v) = s {
            w(v);
        }
    };
    let mut x = x.clone();
    x.sort_keys = false;
    x.books.iter_mut().for_each(|b| {
        wo(&mut b.created_at);
        wo(&mut b.updated_at);
    });
    x.accounts.iter_mut().for_each(|a| {
        wo(&mut a.created_at);
        wo(&mut a.updated_at);
    });
    x.clients.iter_mut().for_each(|c| {
        wo(&mut c.created_at);
        wo(&mut c.updated_at);
    });
    x.batches.iter_mut().for_each(|b| wo(&mut b.created_at));
    x.entries.iter_mut().for_each(|e| {
        w(&mut e.ts);
        wo(&mut e.created_at);
        wo(&mut e.updated_at);
    });
    x.entry_revisions.iter_mut().for_each(|r| wo(&mut r.created_at));
    x
}

fn migrate_book(x: &Export, tb: &TBook) -> BookResult {
    let bid = tb.id.as_str();
    let taccounts: Vec<&TAccount> = x.accounts.iter().filter(|a| a.book_id == bid).collect();
    let chart = Chart {
        by_id: taccounts.iter().map(|a| (a.id.as_str(), *a)).collect(),
    };
    let rules = Rules::new(&chart, &x.cash_flow_pairs, bid);
    let tclients: Vec<&TClient> = x.clients.iter().filter(|c| c.book_id == bid).collect();
    let mut ctx = Ctx {
        book_id: bid,
        accounts: taccounts
            .iter()
            .map(|a| (a.id.clone(), det_v4("account", &format!("{bid}/{}", a.id))))
            .collect(),
        clients_by_id: tclients.iter().map(|c| (c.id.clone(), det_v4("client", &c.id))).collect(),
        clients_by_name: tclients.iter().map(|c| (c.name.clone(), det_v4("client", &c.id))).collect(),
        batches: x.batches.iter().filter(|b| b.book_id == bid).map(|b| (b.id.as_str(), b)).collect(),
        chart,
        rules,
        warnings: vec![],
    };

    // Currencies.
    let mut currencies = Vec::new();
    for c in x.currencies.iter().filter(|c| c.book_id == bid) {
        let mut extra = Extra::new();
        if c.retired {
            put(&mut extra, "retired", Some(true));
            ctx.warn(
                "currencyRetired",
                format!(
                    "currency {} was retired; doublebook currencies cannot retire, kept in extra",
                    c.code
                ),
                Some(&c.code),
            );
        }
        currencies.push(Currency {
            code: c.code.clone(),
            name_english: c.name.clone(),
            name_korean: c.name.clone(),
            symbol: c.symbol.clone(),
            display_decimals: c.display_decimals.clamp(0, 12) as u8,
            extra,
        });
    }

    // Accounts. Sibling order is (sort_order, id) as Terraform lists them; it
    // becomes a dense sortOrder so ties never fall back to the random uuid.
    let mut siblings: HashMap<(Option<String>, String), Vec<&TAccount>> = HashMap::new();
    for a in &taccounts {
        let parent = a.parent_id.clone().filter(|p| ctx.chart.get(p).is_some());
        siblings.entry((parent, a.kind.clone())).or_default().push(a);
    }
    let mut rank: HashMap<&str, i32> = HashMap::new();
    for list in siblings.values_mut() {
        list.sort_by(|a, b| (a.sort_order, &a.id).cmp(&(b.sort_order, &b.id)));
        for (i, a) in list.iter().enumerate() {
            rank.insert(a.id.as_str(), i as i32);
        }
    }
    let mut accounts = Vec::new();
    for a in &taccounts {
        let Some(kind) = account_type(&a.kind) else {
            ctx.warn(
                "badAccountType",
                format!("account type \"{}\" is unknown; account skipped", a.kind),
                Some(&a.id),
            );
            continue;
        };
        let (name_english, name_korean) = names(&a.name, &a.name_i18n);
        let mut extra = Extra::new();
        put(&mut extra, "name", Some(&a.name));
        if let Value::Object(m) = &a.name_i18n {
            for (k, v) in m.iter().filter(|(k, _)| *k != "en" && *k != "ko") {
                put(&mut extra, &format!("name.{k}"), Some(value_text(v)));
            }
        }
        put(&mut extra, "shortName", a.short_name.as_ref());
        put(&mut extra, "path", a.path.as_ref());
        put(&mut extra, "sortOrder", Some(a.sort_order));
        put(&mut extra, "classification", a.classification.as_ref());
        put(&mut extra, "assetCategory", a.asset_category.as_ref());
        put(&mut extra, "liabilityCategory", a.liability_category.as_ref());
        put(&mut extra, "expiryDate", a.expiry_date.as_ref());
        put(&mut extra, "retired", a.retired.then_some(true));
        put(&mut extra, "createdAt", a.created_at.as_ref());
        put(&mut extra, "updatedAt", a.updated_at.as_ref());
        let mut tags = Vec::new();
        if let Some(c) = &a.asset_category {
            tags.push(category_tag(bid, "asset", c));
        }
        if let Some(c) = &a.liability_category {
            tags.push(category_tag(bid, "liability", c));
        }
        let parent_id = match a.parent_id.as_deref() {
            Some(p) if ctx.accounts.contains_key(p) => Some(ctx.accounts[p]),
            Some(p) => {
                ctx.warn("unknownParent", format!("parent {p} does not exist; made top-level"), Some(&a.id));
                None
            }
            None => None,
        };
        let contra_of = match a.contra_of.as_deref() {
            Some(c) if ctx.accounts.contains_key(c) => Some(ctx.accounts[c]),
            Some(c) => {
                ctx.warn("unknownContra", format!("contraOf {c} does not exist; dropped"), Some(&a.id));
                None
            }
            None => None,
        };
        accounts.push(Account {
            unique_id: ctx.accounts[&a.id],
            legacy_id: Some(a.id.clone()),
            kind,
            parent_id,
            sort_order: rank[a.id.as_str()],
            name_english,
            name_korean,
            valid_from: BusinessTime::MIN,
            valid_until: None,
            invalid_ranges: vec![],
            base_currency: a.currency.clone(),
            contra_of,
            is_cash_or_cash_equivalent: ctx.chart.is_cash(a),
            tags,
            extra,
        });
    }

    // Clients.
    let mut clients = Vec::new();
    for c in &tclients {
        let mut extra = Extra::new();
        put(&mut extra, "note", c.note.as_ref());
        put(&mut extra, "retired", c.retired.then_some(true));
        put(&mut extra, "createdAt", c.created_at.as_ref());
        put(&mut extra, "updatedAt", c.updated_at.as_ref());
        clients.push(Client {
            unique_id: ctx.clients_by_id[&c.id],
            legacy_id: Some(c.id.clone()),
            name_english: c.name.clone(),
            name_korean: c.name.clone(),
            valid_from: BusinessTime::MIN,
            valid_until: None,
            invalid_ranges: vec![],
            extra,
        });
    }

    // Entries, in Terraform's order: ts, intraday_seq, id.
    let mut tentries: Vec<(&TEntry, Adt)> = Vec::new();
    for e in x.entries.iter().filter(|e| e.book_id == bid) {
        match parse_adt(&e.ts) {
            Some(adt) => tentries.push((e, adt)),
            None => ctx.warn(
                "badTime",
                format!("entry time \"{}\" is unreadable; entry skipped", e.ts),
                Some(&e.id),
            ),
        }
    }
    tentries.sort_by(|(a, x), (b, y)| (x, a.intraday_seq, &a.id).cmp(&(y, b.intraday_seq, &b.id)));
    let mut postings_of: HashMap<&str, Vec<&TPosting>> = HashMap::new();
    for p in &x.postings {
        postings_of.entry(p.entry_id.as_str()).or_default().push(p);
    }
    let mut revisions_of: HashMap<&str, Vec<&TRevision>> = HashMap::new();
    for r in &x.entry_revisions {
        revisions_of.entry(r.entry_id.as_str()).or_default().push(r);
    }

    // uuidv7: the ULID's own millisecond and random bits, bumped so that
    // entries sharing a minute keep Terraform's order.
    let mut last_in_minute: HashMap<BusinessTime, u64> = HashMap::new();
    let mut entries = Vec::new();
    for (e, adt) in &tentries {
        let Some(time) = adt.to_business_time() else { continue };
        let ulid = e.id.rsplit('-').next().and_then(decode_ulid);
        let fallback_ms = e
            .created_at
            .as_deref()
            .and_then(parse_adt)
            .map(|a| a.kst_to_unix_millis().max(0) as u64)
            .unwrap_or(0);
        let (ms, rand) = ulid.unwrap_or_else(|| (fallback_ms, det_v4("entry", &e.id).as_u128()));
        let ms = match last_in_minute.get(&time) {
            Some(&prev) if ms <= prev => prev + 1,
            _ => ms,
        };
        last_in_minute.insert(time, ms);
        let uid = v7(ms, rand);
        let posts = postings_of.get(e.id.as_str()).cloned().unwrap_or_default();
        let Some(mut entry) = convert_entry(&mut ctx, e, &posts, uid, false) else {
            continue;
        };
        let mut revs = revisions_of.get(e.id.as_str()).cloned().unwrap_or_default();
        revs.sort_by_key(|r| r.revision);
        for (i, r) in revs.iter().enumerate() {
            if r.revision != i as i64 + 1 {
                ctx.warn(
                    "revisionRenumbered",
                    format!("revision {} became {}", r.revision, i + 1),
                    Some(&e.id),
                );
            }
            let before = serde_json::from_value::<TEntry>(r.snapshot.clone()).ok().and_then(|s| {
                let sp: Vec<&TPosting> = s.postings.iter().collect();
                convert_entry(&mut ctx, &s, &sp, uid, true)
            });
            let Some(before) = before else {
                ctx.warn(
                    "badSnapshot",
                    format!("revision {} snapshot is unreadable; revision dropped", r.revision),
                    Some(&e.id),
                );
                continue;
            };
            let recorded_at = r
                .created_at
                .as_deref()
                .and_then(|c| {
                    parse_adt(c)
                        .map(|a| rfc3339_from_unix_millis(a.kst_to_unix_millis()))
                        .or_else(|| Some(c.to_string()))
                })
                .unwrap_or_default();
            entry.revisions.push(EntryRevision {
                revision: entry.revisions.len() as u32 + 1,
                recorded_at,
                author: r.actor.clone(),
                reason: r.reason.clone().unwrap_or_default(),
                before: Box::new(before),
            });
        }
        entries.push(entry);
    }

    // Retired accounts and clients: valid until just after their last
    // non-voided leg (the whole subtree for an account).
    let mut last_account: HashMap<Uuid, BusinessTime> = HashMap::new();
    let mut last_client: HashMap<Uuid, BusinessTime> = HashMap::new();
    for e in entries.iter().filter(|e| e.state != EntryState::Voided) {
        for l in &e.legs {
            let t = last_account.entry(l.account_id).or_insert(e.time);
            *t = (*t).max(e.time);
            if let Some(c) = l.client_id {
                let t = last_client.entry(c).or_insert(e.time);
                *t = (*t).max(e.time);
            }
        }
    }
    let children: HashMap<Uuid, Vec<Uuid>> =
        accounts
            .iter()
            .filter_map(|a| a.parent_id.map(|p| (p, a.unique_id)))
            .fold(HashMap::new(), |mut m, (p, c)| {
                m.entry(p).or_default().push(c);
                m
            });
    let subtree_last = |root: Uuid| {
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        let mut last: Option<BusinessTime> = None;
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(&t) = last_account.get(&id) {
                last = Some(last.map_or(t, |l: BusinessTime| l.max(t)));
            }
            stack.extend(children.get(&id).into_iter().flatten());
        }
        last
    };
    let until = |last: Option<BusinessTime>| Some(last.and_then(BusinessTime::next_minute).unwrap_or(BusinessTime::MIN));
    let retired_accounts: HashSet<&str> = taccounts.iter().filter(|a| a.retired).map(|a| a.id.as_str()).collect();
    for a in &mut accounts {
        if a.legacy_id.as_deref().is_some_and(|l| retired_accounts.contains(l)) {
            a.valid_until = until(subtree_last(a.unique_id));
        }
    }
    let retired_clients: HashSet<&str> = tclients.iter().filter(|c| c.retired).map(|c| c.id.as_str()).collect();
    for c in &mut clients {
        if c.legacy_id.as_deref().is_some_and(|l| retired_clients.contains(l)) {
            c.valid_until = until(last_client.get(&c.unique_id).copied());
        }
    }

    let pairs = x.cash_flow_pairs.iter().filter(|p| p.book_id == bid).count();
    if pairs > 0 {
        ctx.warn(
            "cashFlowPairsDropped",
            format!("{pairs} cash flow pairs dropped; their classifications became LegTypes, their labels are gone"),
            None,
        );
    }
    let notes = x
        .statement_notes
        .iter()
        .filter(|n| n.get("book_id").and_then(Value::as_str) == Some(bid))
        .count();
    if notes > 0 {
        ctx.warn("statementNotesDropped", format!("{notes} statement notes dropped"), None);
    }
    let tax_accounts = taccounts.iter().filter(|a| a.classification.as_deref() == Some("tax")).count();
    if tax_accounts > 0 {
        ctx.warn(
            "taxSection",
            format!("{tax_accounts} accounts classified tax: their cash flows became operating"),
            None,
        );
    }

    let mut extra = Extra::new();
    put(&mut extra, "orgId", tb.org_id.as_ref());
    put(&mut extra, "sortOrder", Some(tb.sort_order));
    put(&mut extra, "createdAt", tb.created_at.as_ref());
    put(&mut extra, "updatedAt", tb.updated_at.as_ref());
    let book = Book {
        unique_id: det_v4("book", bid),
        legacy_id: Some(bid.to_string()),
        name_english: tb.name.clone(),
        name_korean: tb.name.clone(),
        base_currency: tb.base_currency.clone(),
        closing_month: tb.closing_month.clamp(1, 12) as u8,
        currencies,
        accounts,
        clients,
        entries,
        extra,
    };
    let problems = validate_book(&book);
    BookResult {
        book,
        warnings: ctx.warnings,
        problems,
    }
}

impl FromStr for Export {
    type Err = serde_json::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulid_to_v7_keeps_time() {
        let (ms, rand) = decode_ulid("01KZ8CDFRGGDMS1CMA10AFPB8T").unwrap();
        let u = v7(ms, rand);
        assert_eq!(u.get_version_num(), 7);
        let (secs, _) = u.get_timestamp().unwrap().to_unix();
        assert_eq!(secs, ms / 1000);
    }

    #[test]
    fn adt_negative_hour_is_whole_time() {
        let a = parse_adt("2026-08-31T-03:22:00.000").unwrap();
        assert_eq!(a.millis, -(3 * 3_600_000 + 22 * 60_000));
        assert_eq!(a.to_business_time().unwrap().to_string(), "2026-08-31T-04:38");
        let b = parse_adt("2026-08-31T26:01:30.500").unwrap();
        assert!(b.has_sub_minute());
        assert_eq!(b.to_business_time().unwrap().to_string(), "2026-08-31T26:01");
    }

    #[test]
    fn sort_keys_become_wire() {
        assert_eq!(
            parse_sort_key("2026-01-01T33:30:04.000").unwrap().to_wire(),
            "2026-01-01T09:30:04.000"
        );
        assert_eq!(
            parse_sort_key("2026-01-01T20:38:00.000").unwrap().to_wire(),
            "2026-01-01T-03:22:00.000"
        );
        assert_eq!(
            parse_sort_key("2025-12-31T72:00:00.000").unwrap().to_wire(),
            "2025-12-31T48:00:00.000"
        );
    }

    #[test]
    fn det_v4_is_v4_and_stable() {
        let a = det_v4("account", "BK-1/A-0001");
        assert_eq!(a.get_version_num(), 4);
        assert_eq!(a, det_v4("account", "BK-1/A-0001"));
    }
}
