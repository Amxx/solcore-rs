# Solcore standard library snapshot

The `.sol` files in this directory are syntax-migrated ports of the Haskell
reference standard library at revision
`2f372bde2801612814015a22319d0bc51486cbf0`:

```text
https://github.com/argotorg/solcore/tree/2f372bde2801612814015a22319d0bc51486cbf0/std
```

The migration changes surface syntax only; supported semantics remain pinned to
that reference snapshot. The copies under
`crates/parser/tests/fixtures/corpus/ok/std/` are parser fixtures and must remain
byte-for-byte identical to the files here.

This README is Rust-repository metadata; it is not part of the upstream std
snapshot.

## Synchronization policy

Do not apply Rust-only semantic fixes directly to these `.sol` files.

When a shared standard-library defect is found:

1. reproduce it with the Haskell compiler and the upstream std;
2. fix it in the Haskell reference implementation first;
3. record the new upstream revision;
4. re-vendor the complete upstream std change and apply the canonical syntax
   migration to it here and in the parser corpus;
5. verify both backends against the updated semantic snapshot.

Compiler-side code and surface spelling may differ between the Haskell and Rust
implementations, but changes to the library's meaning should not.

## Compatibility decisions

The canonical analysis of Haskell/Rust semantic differences, ABI evidence
gaps, and the recommended owner for each fix is in
[`SEMANTIC_DIFFERENCES.md`](../SEMANTIC_DIFFERENCES.md). ABI metadata support
alone does not make a type externally dispatchable. Keep ABI JSON, selector
spelling, calldata decoding, and result encoding aligned.

## Verification

After every std update, verify at least:

```sh
for file in ABIGeneric.sol Generic.sol StorageGeneric.sol dispatch.sol \
  eip712.sol eip7951.sol opcodes.sol std.sol; do
  cmp "std/$file" "crates/parser/tests/fixtures/corpus/ok/std/$file" || exit 1
done
cargo test -p solcore-parser -p solcore-hir-ty -p solcore-specialize --locked
E2E=1 E2E_REQUIRED=1 cargo test --profile e2e \
  -p solcore-yul --test e2e --locked -- \
  --nocapture --test-threads=1
```

The byte-exact upstream raw-vector metadata is preserved, while both Yul and
Sonatina compile and execute the complete set against Osaka. See
[`tests/e2e/README.md`](../tests/e2e/README.md) for the full validation
commands.
