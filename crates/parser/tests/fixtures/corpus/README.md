# Solcore e136 frontend corpus

This corpus vendors every `.solc` source under `test/examples/` from
[`argotorg/solcore@e1361599`](https://github.com/argotorg/solcore/tree/e13615992388cd7bfd59eef5a2b6f61ddc37da1f/test/examples).
The 497 example paths and their contents are byte-identical to that snapshot.
Sources accepted by the reference frontend live under `ok/test/examples/`;
reference failures and timeouts live under `fail/test/examples/`.

The standard-library sources in `ok/std/` are the matching e136 snapshot. They
are also byte-identical to [`std/`](../../../../../std/); see
[`std/README.md`](../../../../../std/README.md) for the synchronization
policy. The `test/imports/` and `known-diagnostic-gaps/` trees are Rust-specific
regressions and are not part of the reference example snapshot.

## Reference verdicts

[`reference-frontend.tsv`](reference-frontend.tsv) records a fresh run of the
e136 compiler built from the same checkout. Every source was run independently
with a 60-second limit and these options:

```text
sol-core --file <e136>/test/examples/<path> \
  --root <e136> --include <e136>/std \
  --no-specialise --no-gen-dispatch \
  --type-class-resolution legacy \
  --color never --unicode never --diagnostic-format short
```

The snapshot contains 335 passes, 160 failures, and two timeouts. `code` is the
first structured `SCnnnn` diagnostic emitted for a failure; `-` means that no
structured code applies. The two timeout rows are
`cases/tabled-cycle-fail.solc` and `cases/tabled-left-recursive-fail.solc`.
These verdicts describe the legacy frontend with specialization and generated
dispatch disabled, not the full compiler or the tabled resolver.

The two Rust allowance manifests make the comparison mode explicit:

- [`rust-accepted-reference-failures.tsv`](rust-accepted-reference-failures.tsv)
  lists reference failures accepted by the corresponding Rust frontend gate.
- [`rust-rejected-reference-passes.tsv`](rust-rejected-reference-passes.tsv)
  lists diagnostics from Rust's full-frontend gate for reference passes. It
  includes deliberate Rust-only negative import fixtures and phase differences
  caused by enabling generated dispatch.

Do not add a local regression to the reference example trees. Put it in a
dedicated Rust test/fixture tree instead. Parser snapshots are retained only
for reference-failed sources that also exercise a Rust parser diagnostic.
