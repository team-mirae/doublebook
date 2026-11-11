# doublebook

Headless double-entry accounting library with a JSON API, a C ABI and a `doublebook` command-line program.
It holds no storage and draws no UI: any frontend or backend can embed it.

- [docs/SPEC.md](docs/SPEC.md): the model, validation rules and reports.

## Build

```sh
cargo build --release   # target/release/doublebook and target/release/libdoublebook.a
cargo test
```

## Use

Every operation is one JSON request and one JSON response, the same from Rust, C, Go or the CLI.
The requests are listed in `src/api.rs` (`newBook`, `createEntry`, `updateEntry`, `setEntryState`, `createAccount`, ...,
`balanceSheet`, `incomeStatement`, `cashFlowStatement`, ...).

The engine is stateless: each request carries its book, and a request that changes something returns the changed record
for the caller to store. Records imported from another system can keep their old id in `legacyId` and any extra data in `extra`.

```sh
echo '{"op":"fiscalYear","book":{...},"year":2026}' | doublebook call
```

- Rust: depend on the crate and call `doublebook::api::call`, or the typed functions in `book` and `reports`.
- C: `include/doublebook.h` (`doublebook_call`, `doublebook_free`), linking `libdoublebook.a`.
- Go: `bindings/go` wraps it with cgo (`doublebook.Call(request, &result)`).

## License

[CC0 1.0](LICENSE): public domain dedication, no attribution required.
