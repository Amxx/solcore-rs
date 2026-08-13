# solcore-rs

`solcore-rs` is a Rust implementation of [Solcore](https://github.com/argotorg/solcore).

> [!WARNING]
> This project is a work in progress and is not ready for production use.

Try it in the [online playground](https://solcore-rs-preview.solcore-rs-team.workers.dev/).

## Language and compatibility targets

The canonical source-language target is [`syntax.md`](syntax.md). It
intentionally replaces legacy Haskell Solcore spellings with the new Core
surface, so source-syntax compatibility with the Haskell implementation is not
a language goal. The compiler, standard library, examples, and fixtures use the
new `.sol` surface.

For semantics already supported by Core, the comparison baseline remains the
Haskell reference implementation at
[`argotorg/solcore@2f372bde`](https://github.com/argotorg/solcore/tree/2f372bde2801612814015a22319d0bc51486cbf0).
The standard library and the complete 499-source frontend corpus are semantic
ports of that exact revision to the canonical syntax. See
[`SEMANTIC_DIFFERENCES.md`](SEMANTIC_DIFFERENCES.md) for intentional Rust
extensions, shared upstream limitations, and phase-sensitive differences, and
the [corpus README](crates/parser/tests/fixtures/corpus/README.md) for the
reproducible reference verdict configuration.

Semantic compatibility does not imply source-syntax compatibility, production
readiness, or byte-for-byte compiler output. Rust deliberately keeps structured
diagnostics and several safety checks that are stricter than the reference
target, while shared upstream limitations remain explicitly unsupported.

## Build and test

[Rust](https://www.rust-lang.org/tools/install) is required. The checked-in
`rust-toolchain.toml` selects Rust 1.97.0 automatically when using rustup.

```sh
cargo build --workspace --locked
cargo test --workspace --locked
```

## License

Licensed under the [Apache License 2.0](LICENSE).
