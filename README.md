# doublebook

Headless double-entry accounting library for teammirae-app's accountant module, plus a `doublebook` command-line program.

- [docs/SPEC.md](docs/SPEC.md): the model, validation rules and reports.
- [docs/MIGRATION.md](docs/MIGRATION.md): converting Terraform's accountant data, and what changes for its server.

## Build

```sh
cargo build --release   # target/release/doublebook and target/release/libdoublebook.a
cargo test
```

## Use

Every operation is one JSON request and one JSON response, the same from Rust, C, Go or the CLI.
The requests are listed in `src/api.rs` (`newBook`, `createEntry`, `updateEntry`, `setEntryState`, `createAccount`, ...,
`balanceSheet`, `incomeStatement`, `cashFlowStatement`, ...). The engine is stateless: each request carries its book.

```sh
echo '{"op":"fiscalYear","book":{...},"year":2026}' | doublebook call
```

- C: `include/doublebook.h` (`doublebook_call`, `doublebook_free`), linking `libdoublebook.a`.
- Go: `bindings/go` wraps it with cgo (`doublebook.Call(request, &result)`).

## License

[CC0 1.0](LICENSE): public domain dedication, no attribution required.
