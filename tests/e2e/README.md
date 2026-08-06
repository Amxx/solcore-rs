# Backend E2E fixtures

Both the Yul and Sonatina backends run every `**/main.solc` fixture in this
directory. Selector-dispatched fixtures explicitly import both `std.{*}` and
`std.dispatch.{*}`. Expectations live next to the contract function they
exercise:

```solcore
// #[(0, 1) -> 1]
// #[(1, 1) -> 2]
public function add(x: uint256, y: uint256) -> uint256 {
  return Add.add(x, y);
}
```

The directive grammar is `#[(arguments) -> expected]`. Directive values are
typed contextually from the target function's canonical ABI signature. Decimal
and hexadecimal `uint256` values, booleans, and static tuples are supported. An
argument or result with the wrong type or arity is rejected while resolving the
fixture, before any EVM call is made. The execution fixtures deliberately use
only selector ABI types supported by the shared reference std. In particular,
they do not expose primitive `word` or user ADTs directly; those surfaces lack
complete dispatch evidence in the current Haskell snapshot.

State-changing calls use `#[send(arguments)]`. A send directive submits a
transaction, waits for a successful receipt, and preserves its storage changes
for every directive that follows it in the fixture. It has no result
expectation because transaction receipts do not expose EVM returndata. Put a
normal call directive on a later public method to assert the persisted state:

```solcore
// #[send(41)]
public function set(value: uint256) { stored = value; }

// #[() -> 41]
public function readAfterSend() -> uint256 { return stored; }
```

The outer parentheses delimit the argument or result list; another pair is
needed for a tuple value. Thus a single composite argument and result use
double parentheses:

```solcore
// #[((7, 1)) -> (7, 1)]
public function echo(point: (uint256, uint256)) -> (uint256, uint256) {
  return point;
}
```

Top-level tuple returns are flattened into multiple ABI results. The language's
right-nested tuple representation also flattens a nested tuple used as one ABI
parameter: the single argument `((uint256, uint256), uint256)` is written as
`((7, 1, 9))` in a directive. By contrast, two parameters consisting of a pair
and a scalar are written as `((7, 1), 9)`. The complete shared example is in
`composite-values/main.solc`. Normal comments are ignored, while a malformed
comment beginning with `#[` is an error.

For ABI shapes that the compiler metadata cannot describe yet, a fixture may
instead place an upstream-compatible `main.json` next to `main.solc`. The JSON
supplies complete calldata, call value, expected raw returndata or revert
payload, the contract name, and optionally the required EVM version. As in the
upstream runner, every entry is executed once as a real transaction from
`0x1212121212121212121212121212120000000012`; the same execution supplies both
the status/output assertion and the state observed by later entries. Each
vector runs on a fresh, dedicated Anvil instance at its declared version. An
omitted `evmVersion` means Prague, matching upstream; vectors that require
Osaka must say so explicitly. Yul compilation uses that same target. The
current Sonatina dependency only supports Osaka, so its runner rejects other
targets explicitly instead of emitting Osaka bytecode for an older runtime.
This is also the migration format for Solcore's dispatch fixtures with dynamic
arrays or ADTs.

Each case is lowered by the selected backend, compiled to EVM creation
bytecode, deployed to Anvil, and called through the generated ABI selector.
Set `E2E=1` to run execution tests. `E2E_PIPELINE_ONLY=1` stops after backend
code generation; `E2E_REQUIRED=1` makes missing tools an error. Anvil defaults
to the Osaka hardfork for directive fixtures; `ANVIL_HARDFORK` can override it
for an alternate Yul runtime. Raw vectors always use their own `evmVersion`.

For local optimized runs, use the workspace's E2E profile. It uses moderate
optimization (`opt-level = 2`) without LTO, keeping execution representative
while avoiding the native release profile's link-time optimization cost:

```sh
E2E=1 E2E_REQUIRED=1 cargo test --profile e2e \
  -p solcore-yul -p solcore-sonatina --test e2e --locked -- \
  --nocapture --test-threads=1
```
