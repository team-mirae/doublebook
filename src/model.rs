//! The data model of `docs/SPEC.md`. JSON field names are the spec's.

use std::collections::BTreeMap;

use bigdecimal::BigDecimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::time::{BusinessTime, BusinessTimeRange};

/// Arbitrary-precision exact decimal, written as a JSON string.
pub type Dec = BigDecimal;

/// Free strings carried verbatim; the engine never reads them.
pub type Extra = BTreeMap<String, String>;

pub mod dec_str {
    use super::Dec;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::str::FromStr;

    pub fn serialize<S: Serializer>(d: &Dec, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&d.to_plain_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Dec, D::Error> {
        let s = String::deserialize(d)?;
        parse(&s).map_err(serde::de::Error::custom)
    }

    /// Accepts plain decimal strings only: optional '-', digits, optional fraction.
    pub fn parse(s: &str) -> Result<Dec, String> {
        let body = s.strip_prefix('-').unwrap_or(s);
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        let ok = !int.is_empty()
            && int.bytes().all(|c| c.is_ascii_digit())
            && (!body.contains('.') || (!frac.is_empty() && frac.bytes().all(|c| c.is_ascii_digit())));
        if !ok {
            return Err(format!(
                "invalid decimal \"{s}\": expected a plain decimal string such as \"1500\" or \"-12.50\""
            ));
        }
        Dec::from_str(s).map_err(|e| format!("invalid decimal \"{s}\": {e}"))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub unique_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
    pub name_english: String,
    pub name_korean: String,
    pub base_currency: String,
    pub closing_month: u8,
    #[serde(default)]
    pub currencies: Vec<Currency>,
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub clients: Vec<Client>,
    #[serde(default)]
    pub entries: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Currency {
    pub code: String,
    pub name_english: String,
    pub name_korean: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default)]
    pub display_decimals: u8,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AccountType {
    Asset,
    Liability,
    Income,
    Expense,
    Equity,
}

impl AccountType {
    /// Debit-normal types (Asset, Expense) gain on debit.
    pub fn debit_normal(self) -> bool {
        matches!(self, AccountType::Asset | AccountType::Expense)
    }

    /// Tree order of report sections.
    pub fn report_rank(self) -> u8 {
        match self {
            AccountType::Asset => 0,
            AccountType::Liability => 1,
            AccountType::Equity => 2,
            AccountType::Income => 3,
            AccountType::Expense => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub unique_id: Uuid,
    pub name_english: String,
    pub name_korean: String,
    pub name_slug: String,
}

pub type AccountTag = Tag;
pub type LegTag = Tag;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub unique_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: AccountType,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    #[serde(default)]
    pub sort_order: i32,
    pub name_english: String,
    pub name_korean: String,
    pub valid_from: BusinessTime,
    #[serde(default)]
    pub valid_until: Option<BusinessTime>,
    #[serde(default)]
    pub invalid_ranges: Vec<BusinessTimeRange>,
    pub base_currency: String,
    #[serde(default)]
    pub contra_of: Option<Uuid>,
    #[serde(default)]
    pub is_cash_or_cash_equivalent: bool,
    #[serde(default)]
    pub tags: Vec<AccountTag>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryType {
    Normal,
    Closing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryState {
    Draft,
    Final,
    Voided,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub unique_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub remarks: String,
    pub time: BusinessTime,
    pub legs: Vec<EntryLeg>,
    #[serde(rename = "type")]
    pub kind: EntryType,
    pub state: EntryState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revisions: Vec<EntryRevision>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LegSide {
    Debit,
    Credit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegType {
    Operating,
    Financing,
    Investing,
    FxEffects,
    Other,
}

impl LegType {
    /// Cash flow statement section order.
    pub const SECTIONS: [LegType; 5] = [
        LegType::Operating,
        LegType::Investing,
        LegType::Financing,
        LegType::FxEffects,
        LegType::Other,
    ];
    /// Spec order.
    pub const ALL: [LegType; 5] = [
        LegType::Operating,
        LegType::Financing,
        LegType::Investing,
        LegType::FxEffects,
        LegType::Other,
    ];
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegMetadata {
    #[serde(rename = "type", default)]
    pub kind: Option<LegType>,
    #[serde(default)]
    pub tags: Vec<LegTag>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryLeg {
    pub account_id: Uuid,
    pub side: LegSide,
    #[serde(default)]
    pub client_id: Option<Uuid>,
    #[serde(with = "dec_str")]
    pub book_amount: Dec,
    #[serde(with = "dec_str")]
    pub cash_amount: Dec,
    #[serde(with = "dec_str")]
    pub original_amount: Dec,
    pub original_currency: String,
    #[serde(default)]
    pub is_tax_relevant: bool,
    #[serde(default)]
    pub metadata: LegMetadata,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryRevision {
    pub revision: u32,
    /// RFC 3339 wall-clock time, not BusinessTime.
    pub recorded_at: String,
    pub author: String,
    pub reason: String,
    pub before: Box<Entry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub unique_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
    pub name_english: String,
    pub name_korean: String,
    pub valid_from: BusinessTime,
    #[serde(default)]
    pub valid_until: Option<BusinessTime>,
    #[serde(default)]
    pub invalid_ranges: Vec<BusinessTimeRange>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl EntryLeg {
    /// Natural-balance contribution of this leg for an account of `kind`.
    pub fn natural(&self, kind: AccountType) -> Dec {
        natural(kind, self.side, &self.book_amount)
    }
}

pub fn natural(kind: AccountType, side: LegSide, amount: &Dec) -> Dec {
    let gains = (side == LegSide::Debit) == kind.debit_normal();
    if gains { amount.clone() } else { -amount.clone() }
}
