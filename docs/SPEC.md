# Spec

Notation: `field: Type` per line, `// comment` for constraints. `T?` is optional, `T[]` a list.

Every record that came from another system can carry its old id in `legacyId: string?`, and `extra: map<string, string>` holds
free strings the engine never reads (Book, Currency, Account, Client, Entry, EntryLeg). Both are carried verbatim and change no rule.

## Time

### BusinessTime

`yyyymmdd hhmm`, where `hh` is a signed 32-bit integer (`i32`) and `mm` is `[0, 59]`.

- Text form: `YYYY-MM-DDTHH:MM`, the hour signed and at least two digits: `2026-08-30T26:01`, `2026-08-31T-04:38`.
- The minute is always added to the hour: `-04:38` is -4 hours plus 38 minutes, 3h22m before midnight.
- Range: `0000-01-01T-2147483648:00` to `9999-12-31T2147483647:59`.
- The date always wins a comparison: 1/15 99:00 is before 1/16 -99:00. Then hour, then minute.
- Entries at the same BusinessTime are ordered by their uuidv7 `uniqueId` (this is why entries use v7).

### BusinessTimeRange

```text
from:  BusinessTime  // inclusive
until: BusinessTime  // exclusive
```

## Book

```text
uniqueId:     uuidv4
nameEnglish:  string
nameKorean:   string
baseCurrency: string      // code of one of this book's currencies; immutable once the book has entries
closingMonth: u8 [1, 12]  // last month of the fiscal year
currencies:   Currency[]
accounts:     Account[]
clients:      Client[]
entries:      Entry[]
```

## Currency

```text
code:            string      // 2-12 ASCII letters or digits, unique per book, immutable once created
nameEnglish:     string
nameKorean:      string
symbol:          string?     // e.g. US$
displayDecimals: u8 [0, 12]  // presentation only: never rounds or limits what is stored
```

- The base currency comes with the book. Every other currency is added explicitly.
- No fx rate is ever looked up: a leg in a foreign currency carries its `bookAmount` explicitly.

## Accounts

### Account

```text
uniqueId:               uuidv4
type:                   AccountType          // immutable once created
parentId:               uuid?                // same type as this account; null for a top-level account
sortOrder:              i32                  // display order among siblings
nameEnglish:            string
nameKorean:             string
validFrom:              BusinessTime
validUntil:             BusinessTime | null  // exclusive
invalidRanges:          BusinessTimeRange[]
baseCurrency:           string               // default book currency, must be explicit; immutable once the account has legs
contraOf:               uuid?                // an account of the same type this one is a contra of,
                                             // e.g. accumulated depreciation against its asset
isCashOrCashEquivalent: boolean              // asset accounts only, immutable once the account has legs;
                                             // legs on these accounts are the cash itself,
                                             // legs elsewhere are what the cash moved against
tags:                   AccountTag[]
```

- Accounts form a tree per type. Only leaves take legs: an account that has legs (voided entries included) cannot be given a child.
- Tree order: Asset, Liability, Equity, Income, Expense; siblings by `sortOrder`, then `uniqueId`.
- The tree must not contain cycles.
- Retiring: an account is retired by setting `validUntil` to the border of when it was valid.
- An account is only valid where its parent is valid as well.

### AccountType

`Asset` | `Liability` | `Income` | `Expense` | `Equity`

### AccountTag

```text
uniqueId:    uuidv4
nameEnglish: string
nameKorean:  string
nameSlug:    string
```

Same shape as LegTag; replaces fixed account categories. Example: `Receivable` / `매출채권` / `receivable`.

## Entries

### Entry

```text
uniqueId:    uuidv7
name:        string
description: string
remarks:     string
time:        BusinessTime
legs:        EntryLeg[]
type:        EntryType
state:       EntryState
revisions:   EntryRevision[]  // oldest first
```

### EntryType

`normal` | `closing`

### EntryState

| Value | Meaning |
|---|---|
| `draft` | still computed; only a flag to mark it non-final |
| `final` | |
| `voided` | soft delete |

### EntryLeg

```text
accountId:        uuid
side:             LegSide
clientId:         uuid?
bookAmount:       BigDec       // zero or positive, in the book's base currency
cashAmount:       BigDec       // how much of bookAmount is cash, [0, bookAmount];
                               // on an isCashOrCashEquivalent account this is the full bookAmount
originalAmount:   BigDec       // zero or positive
originalCurrency: string       // must equal the account's baseCurrency
isTaxRelevant:    boolean      // income and expense legs: counts toward taxable instead of nonTaxable
                               // in the income statement's tax split; moot on asset, liability and equity legs
metadata:         LegMetadata
```

### LegSide

`debit` | `credit`

### LegMetadata

```text
type: LegType?   // required on non-cash legs, empty on legs whose account has isCashOrCashEquivalent;
                 // a cash movement is classified by the LegType of the non-cash legs it moved against
tags: LegTag[]
```

### LegType

`operating` | `financing` | `investing` | `fx_effects` | `other`

### LegTag

```text
uniqueId:    uuidv4
nameEnglish: string
nameKorean:  string
nameSlug:    string
```

A string wrapper object. Example: `R&D` / `연구개발비` / `research-and-development`.

### EntryRevision

```text
revision:   u32        // 1, 2, 3, ... per entry
recordedAt: string     // RFC 3339 UTC wall-clock time of the change, not BusinessTime
author:     string
reason:     string     // required: why the change was made
before:     Entry      // snapshot of the entry as it was before this change, without its revisions
```

- Every change to an entry, including a state change (draft, final, voided and back), appends one revision.
- The `uniqueId` never changes.

## Client

```text
uniqueId:      uuidv4
nameEnglish:   string
nameKorean:    string
validFrom:     BusinessTime
validUntil:    BusinessTime | null  // exclusive
invalidRanges: BusinessTimeRange[]
```

## Validation

An entry is checked as a whole and every problem is reported at once, not just the first.
A voided entry is not checked, so voiding always succeeds; making it draft or final again checks it in full.
An entry's `uniqueId` is a uuidv7.

1. An entry has at least two legs.
2. Sum of debit `bookAmount` equals sum of credit `bookAmount`.
3. Sum of debit `cashAmount` equals sum of credit `cashAmount`.
4. `0 <= cashAmount <= bookAmount` on every leg.
5. A zero entry is allowed: every amount may be 0.
6. Each leg posts to a leaf account of the book.
7. The account is valid at the entry's time: on or after `validFrom`, before `validUntil`, and outside every invalid range (its ancestors too).
8. `originalCurrency` equals the account's `baseCurrency` and is one of the book's currencies.
9. For an account in the base currency, `originalAmount` equals `bookAmount`.
10. If `clientId` is set, the client exists in the book and is valid at the entry's time.
11. `contraOf`, if set, names an account of the same type in the same book, and not the account itself.
12. On a leg whose account has `isCashOrCashEquivalent`, `cashAmount` equals `bookAmount`.
13. `isCashOrCashEquivalent` is only true on asset accounts.
14. A closing entry is rejected if any draft entry exists at or before its time.
    Likewise, an entry cannot become a draft (created as one, edited to one, or moved in time while a draft)
    at or before the time of an existing closing entry. Voided entries count for neither.
15. A leg whose account has `isCashOrCashEquivalent` has no `LegType`; every other leg has one.

### Validity changes

- Changing `validFrom`, `validUntil` or `invalidRanges` of an account or client (including retiring it) is rejected if any non-voided leg would then fall outside its validity.
- For an account this includes the legs of every account under it, since a child is only valid where its parent is.
- Moving an account to another parent is checked the same way.

## Reports

Every report is a pure function of the book: same book and same arguments, same result.
Amounts are in the book's base currency unless a field says otherwise. Sums are exact; nothing is rounded.

### Scope

- A point-in-time report takes `at: BusinessTime` and reads every entry with `time <= at`.
- A period report takes `range: BusinessTimeRange` and reads entries with `range.from <= time < range.until` for its movements.
  Its opening figures read entries with `time < range.from`, its closing figures entries with `time < range.until`.
- Entries are processed in order: time first, then uuidv7.
- Voided entries are never read and never cause an error.
- Draft entries are read like final ones, unless `requireFinal` is set.

`requireFinal: boolean`: when true, everything the report covers must be final, including all history before it.
The report fails with `DraftInScope` if any draft entry has `time <= at` (point-in-time) or `time < range.until` (period),
whether or not it would change a figure.

```text
DraftInScope
  entryIds: uuid[]  // every offending draft, in entry order
```

### Signs

- Every amount is shown positive in its account's normal direction; contra accounts are negative.
- Natural balance of an account: `debit - credit` for Asset and Expense, `credit - debit` for Liability, Equity and Income.
- A contra account has the same type as the account it is contra of but moves the other way, so its natural balance comes out negative (accumulated depreciation, sales returns) and nets against its parent without special handling.
- A negative figure on a non-contra account therefore means an abnormal balance, such as an overdrawn bank account.
- Debit and credit columns, inflows and outflows are always shown as positive sums.
- A parent account's balance is the sum of its children's.

### Which accounts appear

- A point-in-time report shows every account with a non-zero balance at `at`, whether or not it is valid then.
- A period report shows every account with a non-zero opening balance, closing balance or movement in range.
- A parent appears when any account under it appears.

```text
AccountNode
  accountId:       uuid
  balance:         BigDec         // natural balance, this account and everything under it
  originalBalance: BigDec?        // leaves in a foreign currency: the balance in the account's own currency
  children:        AccountNode[]  // in sortOrder
```

### accountBalance(book, accountId, at, requireFinal) -> AccountNode

One account and its subtree.

### trialBalance(book, at, requireFinal) -> TrialBalance

```text
TrialBalance
  rows:        TrialBalanceRow[]  // every leaf with a non-zero balance at `at`, in tree order
  totalDebit:  BigDec
  totalCredit: BigDec             // always equal to totalDebit

TrialBalanceRow
  accountId: uuid
  debit:     BigDec  // sum of debit bookAmount
  credit:    BigDec  // sum of credit bookAmount
  balance:   BigDec  // natural balance
```

### balanceSheet(book, at, requireFinal) -> BalanceSheet

```text
BalanceSheet
  assets:                    AccountNode[]
  liabilities:               AccountNode[]
  equity:                    AccountNode[]
  unclosedNetIncome:         BigDec  // natural Income balance - natural Expense balance at `at`;
                                     // what closing entries have not yet moved into equity
  cash:                      BigDec  // sum of all isCashOrCashEquivalent accounts
  totalAssets:               BigDec
  totalLiabilitiesAndEquity: BigDec  // liabilities + equity + unclosedNetIncome, always equal to totalAssets
```

### incomeStatement(book, range, requireFinal) -> IncomeStatement

Closing entries are excluded, so a closed period still shows what it earned.

```text
IncomeStatement
  income:     AccountNode[]      // movement in range, not balance
  expenses:   AccountNode[]
  byLegType:  LegTypeSubtotal[]  // income and expense legs grouped by LegType
  netIncome:  BigDec             // income - expenses
  taxable:    TaxSplit           // income and expense legs with isTaxRelevant
  nonTaxable: TaxSplit           // income and expense legs without it;
                                 // taxable.net + nonTaxable.net = netIncome

TaxSplit
  income:   BigDec
  expenses: BigDec
  net:      BigDec  // income - expenses

LegTypeSubtotal
  type:     LegType
  income:   BigDec
  expenses: BigDec
  net:      BigDec
```

- Either part of the tax split may be negative or larger than `netIncome`, and that is never an error: `netIncome` 100 can split into taxable 105 and nonTaxable -5.
- Built for sole proprietorships: there is no income tax expense of the business itself, so splitting net income by `isTaxRelevant` gives the taxable business income directly.

### cashFlowStatement(book, range, requireFinal) -> CashFlowStatement

Built from non-cash legs only. A credit leg's `cashAmount` is an inflow, a debit leg's an outflow; the section is the leg's `LegType`.
Cash legs only make up the opening and closing cash, so transfers between cash accounts show as nothing.

```text
CashFlowStatement
  openingCash: BigDec             // cash accounts, entries with time < range.from
  sections:    CashFlowSection[]  // operating, investing, financing, fx_effects, other; empty sections included
  netChange:   BigDec
  closingCash: BigDec             // always openingCash + netChange

CashFlowSection
  type:    LegType
  lines:   CashFlowLine[]  // one per counter leaf account that moved cash, in tree order
  inflow:  BigDec
  outflow: BigDec
  net:     BigDec

CashFlowLine
  accountId: uuid
  inflow:    BigDec
  outflow:   BigDec
  net:       BigDec
```

### changesInEquity(book, range, requireFinal) -> ChangesInEquity

```text
ChangesInEquity
  rows:                     EquityRow[]  // every equity leaf with a balance or movement, in tree order
  openingUnclosedNetIncome: BigDec
  closingUnclosedNetIncome: BigDec
  openingTotal:             BigDec       // equity + unclosed net income
  closingTotal:             BigDec

EquityRow
  accountId:          uuid
  opening:            BigDec
  fromClosingEntries: BigDec  // movement by closing entries in range
  otherMovements:     BigDec  // every other movement in range
  closing:            BigDec
```

### clientBalances(book, at, requireFinal, accountId?) -> ClientBalance[]

Per client, per leaf, the natural balance of legs carrying that `clientId`. `accountId` limits it to one subtree.
Legs without a client are left out.

```text
ClientBalance
  clientId: uuid
  accounts: AccountNode[]
  total:    BigDec
```

### generalLedger(book, accountId, range, requireFinal) -> GeneralLedger

Every leg in range on the account or anything under it, with a running balance.

```text
GeneralLedger
  accountId: uuid
  opening:   BigDec
  lines:     LedgerLine[]
  closing:   BigDec

LedgerLine
  entryId:          uuid
  time:             BusinessTime
  state:            EntryState    // draft or final
  legAccountId:     uuid
  side:             LegSide
  bookAmount:       BigDec
  originalAmount:   BigDec
  originalCurrency: string
  clientId:         uuid?
  runningBalance:   BigDec        // natural balance of the ledger's account after this line
```

### journal(book, range, requireFinal) -> Entry[]

Non-voided entries in range, in entry order, as stored (revisions included).

### legTagSummary(book, range, requireFinal) -> LegTagTotal[]

Movement in range per LegTag, across all accounts. A leg with several tags counts toward each of them.

```text
LegTagTotal
  tagId:  uuid
  debit:  BigDec
  credit: BigDec
```

### fiscalYear(book, year: i32) -> BusinessTimeRange

Helper, not a report. The fiscal year that ends with `closingMonth` of `year`:

```text
from:  first day of the month after closingMonth in year - 1, hh i32::MIN, mm 0
until: first day of the month after closingMonth in year,     hh i32::MIN, mm 0
```

Every entry dated inside the year is in it, whatever its hour. `closingMonth` 12, year 2026: 2026-01-01 to 2027-01-01.

## JSON

Field names as written in this spec. Amounts (`BigDec`) are plain decimal strings (`"1500"`, `"-12.50"`), never JSON numbers.
Enum values: AccountType `Asset` `Liability` `Income` `Expense` `Equity`; EntryType `normal` `closing`;
EntryState `draft` `final` `voided`; LegSide `debit` `credit`; LegType `operating` `financing` `investing` `fx_effects` `other`.
