# Migrating from Terraform's accountant

`doublebook migrate-terraform DIR` converts a Terraform accountant export into doublebook books.
The conversion is deterministic: the same export always gives the same books, ids included.

```sh
scripts/export-terraform.sh postgres "$DATABASE_URL" export/   # live Terraform (acct_* tables)
doublebook migrate-terraform export/ > migrated.json
```

The output is `{"books": [{"book", "warnings", "problems"}]}`. `problems` is `validate_book` on the result and should be empty;
`warnings` lists everything the target model cannot hold exactly. Nothing is dropped silently: data without a field lands in `extra`,
under keys starting `terraform.` (metadata keys of postings are carried under their own names).

A magic-company SQLite backup (time columns stored as +24-biased sort keys) works too:
`scripts/export-terraform.sh sqlite backup.db export/` then `doublebook migrate-terraform export/ --sort-keys`.

## Parity

Run on the 2026-10-08 magic-company backup (5 books, 3,209 entries, 11,110 postings), every figure below matched Terraform's own Go
engine (`accountant/engine`) for each book and each calendar year 2019-2031: 390 checks, no difference.

- the natural balance of every account at each year end
- net income per year, closing batches excluded
- cash flow opening, closing and net change per year
- cash flow per section per year (Terraform's `tax` section compared as operating, `fx` as `fx_effects`)

The harness is a small Go program over `accountant/engine` and `reports.ComputeCashFlow`; rerun it on a fresh export before switching over.

## Mapping

| Terraform | doublebook |
|---|---|
| book `id`, `name` | `legacyId`; `nameEnglish` = `nameKorean` = name. `org_id`, `sort_order`, timestamps in `extra` |
| currency | same code, names = name, symbol, decimals. `retired` in `extra` (warning: doublebook currencies cannot retire) |
| account `A-0001` etc. | uuid derived from book + id; `legacyId` keeps `A-0001` |
| account `name`, `name_i18n` | `nameEnglish` = i18n `en` else name; `nameKorean` = i18n `ko` else name. `name`, other languages, `short_name`, `path` in `extra` |
| `sort_order` | dense rank among siblings by (`sort_order`, id), Terraform's chart order. Original in `extra` |
| `asset_category`, `liability_category` | an AccountTag each (`cash`, `receivable`, `credit-card`, ...), as set on the account |
| cash perimeter (category cash or cash_equivalent, inherited, contraOf one hop) | `isCashOrCashEquivalent` |
| `classification`, `expiry_date` | `extra` only; classification lives on legs now |
| `retired` | `validUntil` = the minute after the last non-voided leg in its subtree (`validFrom` when it has none) |
| client | uuid derived from id; names = name; `note`, `retired` in `extra`; retired → `validUntil` as for accounts |
| entry `J-<ULID>` | uuidv7 from the ULID's own millisecond and random bits, bumped by 1 ms where needed so entries sharing a minute keep Terraform's (`ts`, `intraday_seq`, `id`) order |
| `ts` | BusinessTime floored to the minute (warning when seconds are dropped); exact original in `extra` |
| `payee`, `detail`, `memo` | `name`, `description`, `remarks` |
| `status` posted / draft / void | `final` / `draft` / `voided`; `prior_status` in `extra` |
| batch | gone. Kind `closing` → EntryType `closing`; batch id, kind, label, note, generator in the entry's `extra` |
| entry `client_id`, posting `metadata.client` | leg `clientId`: the posting's own client (id or exact name) else the entry's |
| posting `amount`, `base_amount`, `currency` | `originalAmount`, `bookAmount`, `originalCurrency`. A negative pair flips the side |
| posting `metadata` | leg `extra`, verbatim; posting id under `terraform.postingId` |
| — | `cashAmount`: Terraform's cash flow allocation per entry, ported exactly (merge per account and revaluation flag, largest first, restatements first, greedy). Cash legs carry their full amount |
| — | `LegType` on non-cash legs: the cash flow section that leg's cash landed in (pair rule classification, `fx` for revaluations, else the account's inherited bucket). Legs without cash take the account's bucket. `tax` → `operating`, `fx` → `fx_effects` |
| — | `isTaxRelevant`: true on every income and expense leg, taxes and levies included (they are deductible costs). Edit legs that should count as non-taxable |
| revisions | `author` = actor, `reason`, `recordedAt` = `created_at` read as KST and written as RFC 3339 UTC, `before` = the snapshot converted like an entry. A snapshot naming a deleted account keeps a derived account id |
| cash flow pairs | dropped. Their classifications already shaped `LegType`; their labels are gone |
| statement notes | dropped |

## Times

Terraform writes a negative time with the sign on the whole time: `T-03:22` is 3h22m before midnight.
doublebook adds the minute to a signed hour, so the same moment is `T-04:38`. The migration converts the moment exactly;
only the text differs. Terraform's hours run -24 to 48; doublebook's span all of i32.

## What changes for the Terraform server

- Storage: doublebook is stateless. Each call carries the book; a call that changes something returns the changed record to persist.
  The `acct_*` tables need a new shape for the doublebook model (or one JSON document per book).
- Calls: link `libdoublebook.a` through cgo with `bindings/go` and send the requests in `src/api.rs`.
  Every request and response is JSON, amounts as strings.
- UI and API: ids become UUIDs; `legacyId` resolves old links. Batches, generators (amortization, recurring, closing,
  fx revaluation), cash flow pairs and statement notes have no engine support: closing and revaluation entries are posted as
  ordinary entries, closing ones with type `closing`.
