# solcore-rs

`solcore-rs` is a Rust implementation of [Solcore](https://github.com/argotorg/solcore).

> [!WARNING]
> This project is a work in progress and is not ready for production use.

Try it in the [online playground](https://solcore-rs-preview.solcore-rs-team.workers.dev/).

## Compatibility target

The current compatibility target is the Haskell reference implementation at
[`argotorg/solcore@e1361599`](https://github.com/argotorg/solcore/tree/e13615992388cd7bfd59eef5a2b6f61ddc37da1f).
The standard library and the complete 497-source reference frontend corpus are
vendored from that exact revision. See
[`SEMANTIC_DIFFERENCES.md`](SEMANTIC_DIFFERENCES.md) for intentional Rust
extensions, shared upstream limitations, and phase-sensitive differences, and
the [corpus README](crates/parser/tests/fixtures/corpus/README.md) for the
reproducible reference verdict configuration.

Compatibility does not imply production readiness or byte-for-byte compiler
output. Rust deliberately keeps structured diagnostics and several safety
checks that are stricter than e136, while shared e136 limitations remain
explicitly unsupported.

## Build and test

[Rust](https://www.rust-lang.org/tools/install) is required. The checked-in
`rust-toolchain.toml` selects Rust 1.97.0 automatically when using rustup.

```sh
cargo build --workspace --locked
cargo test --workspace --locked
```

## License

Licensed under the [Apache License 2.0](LICENSE).
