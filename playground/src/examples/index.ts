export interface ExampleFile {
  path: string;
  content: string;
}

export interface PlaygroundExample {
  id: string;
  name: string;
  description: string;
  entry: string;
  files: ExampleFile[];
}

export const examples: PlaygroundExample[] = [
  {
    id: "contract-output",
    name: "Hello contract",
    description: "A small contract that emits Hull, Yul, Sonatina IR, and ABI JSON.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import * from std;
import * from std.dispatch;

contract Answer {
    function main() public returns (uint256) {
        return uint256(42);
    }
}
`,
      },
    ],
  },
  {
    id: "std-usage",
    name: "Std usage",
    description: "Calls a standard-library trait method directly; the + operator desugars to the same call.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import * from std;
import * from std.dispatch;

contract Calculator {
    // The + operator is not compiler magic: it desugars to the same
    // standard-library trait method called explicitly below.
    function viaOperator(a: uint256, b: uint256) public returns (uint256) {
        return a + b;
    }

    function viaTrait(a: uint256, b: uint256) public returns (uint256) {
        return Num.add(a, b);
    }
}
`,
      },
    ],
  },
  {
    id: "trait",
    name: "Trait",
    description: "A trait implemented for a custom enum drives a stored on-chain state machine.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import * from std;
import * from std.dispatch;
import * from std.Generic;
import * from std.StorageGeneric;

trait Toggle<a> {
    function toggle(value: a) returns (a);
}

enum Switch {
    Off,
    On
}

impl Toggle<Switch> {
    function toggle(value: Switch) returns (Switch) {
        match (value) {
            case Switch.Off { return Switch.On; }
            case Switch.On { return Switch.Off; }
        }
    }
}

contract LightSwitch {
    state : Switch;

    constructor() {
        state = Switch.Off;
    }

    function flip() public {
        state = Toggle.toggle(state);
    }

    function isOn() public returns (bool) {
        match (state) {
            case Switch.On { return true; }
            default { return false; }
        }
    }
}
`,
      },
    ],
  },
  {
    id: "generics",
    name: "Generics",
    description: "A where-constrained generic max works for a user-defined Version type via its Ord impl.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import * from std;
import * from std.dispatch;
import * from std.Generic;
import * from std.StorageGeneric;

// One generic maximum for every ordered type, user-defined included.
// Candidate standard-library inventory: a generic max belongs in std
// eventually.
function max<a>(x: a, y: a) returns (a) where a: Ord {
    if (x > y) { return x; }
    return y;
}

enum Version {
    Version(uint256, uint256)
}

function major(v: Version) returns (uint256) {
    match (v) {
        case Version.Version(value, _) { return value; }
    }
}

function minor(v: Version) returns (uint256) {
    match (v) {
        case Version.Version(_, value) { return value; }
    }
}

impl Eq<Version> {
    function eq(a: Version, b: Version) returns (bool) {
        return major(a) == major(b) && minor(a) == minor(b);
    }
}

// Ord requires Eq: the compiler checks the trait hierarchy.
impl Ord<Version> {
    function gt(a: Version, b: Version) returns (bool) {
        if (major(a) == major(b)) { return minor(a) > minor(b); }
        return major(a) > major(b);
    }
}

contract Registry {
    newest : Version;

    constructor() {
        newest = Version.Version(uint256(0), uint256(0));
    }

    function publish(maj: uint256, min: uint256) public {
        newest = max(newest, Version.Version(maj, min));
    }

    function newestMajor() public returns (uint256) {
        return major(newest);
    }

    function newestMinor() public returns (uint256) {
        return minor(newest);
    }
}
`,
      },
    ],
  },
  {
    id: "option",
    name: "Option",
    description: "A reusable Option module and a splitter contract: checked division returns an Option instead of reverting.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import * from std;
import * from std.dispatch;
import {Option, checkedDiv, unwrapOr} from option;

contract Splitter {
    // Each recipient's equal share of the pot; zero recipients yields
    // zero instead of reverting on division.
    function share(pot: uint256, recipients: uint256) public returns (uint256) {
        return unwrapOr(checkedDiv(pot, recipients), uint256(0));
    }

    // What is left over after handing out equal shares.
    function remainder(pot: uint256, recipients: uint256) public returns (uint256) {
        match (checkedDiv(pot, recipients)) {
            case Option.Some(perRecipient) { return pot - perRecipient * recipients; }
            default { return pot; }
        }
    }
}
`,
      },
      {
        path: "option.sol",
        content: `// Candidate standard-library inventory: this module should disappear once
// std provides Option.
import * from std;
import * from std.Generic;
import * from std.StorageGeneric;

export { Option(*), checkedDiv, unwrapOr, contains };

enum Option<a> {
    None,
    Some(a)
}

// Division by zero yields Option.None instead of reverting.
function checkedDiv(a: uint256, b: uint256) returns (Option<uint256>) {
    if (b == uint256(0)) {
        return Option.None;
    }
    return Option.Some(a / b);
}

function unwrapOr<a>(option: Option<a>, orElse: a) returns (a) {
    match (option) {
        case Option.Some(value) { return value; }
        default { return orElse; }
    }
}

function contains<a>(option: Option<a>, value: a) returns (bool) where a: Eq {
    match (option) {
        case Option.Some(inner) { return inner == value; }
        default { return false; }
    }
}
`,
      },
    ],
  },
  {
    id: "pattern-matching",
    name: "Pattern matching",
    description: "An escrow whose lifecycle is an enum stored in a contract field, driven by match.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import * from std;
import * from std.dispatch;
import * from std.Generic;
import * from std.StorageGeneric;

// An escrow whose lifecycle is a sum type stored in a contract field.
enum Phase {
    AwaitingPayment,
    Funded(uint256),
    Released(uint256)
}

contract Escrow {
    phase: Phase;

    constructor() {
        phase = Phase.AwaitingPayment;
    }

    function deposit(amount: uint256) public {
        match (phase) {
            case Phase.AwaitingPayment {
                phase = Phase.Funded(amount);
            }
            default {
                require(false, "already funded");
            }
        }
    }

    function release() public returns (uint256) {
        match (phase) {
            case Phase.Funded(amount) {
                phase = Phase.Released(amount);
                return amount;
            }
            default {
                require(false, "nothing to release");
                return uint256(0);
            }
        }
    }

    // 0 = awaiting payment, 1 = funded, 2 = released
    function status() public returns (uint256) {
        match (phase) {
            case Phase.AwaitingPayment { return uint256(0); }
            case Phase.Funded(_) { return uint256(1); }
            case Phase.Released(_) { return uint256(2); }
        }
    }
}
`,
      },
    ],
  },
  {
    id: "mini-nft",
    name: "Mini NFT",
    description: "An NFT with typed ownership: tokens either have an owner or do not exist, with no zero-address sentinels.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import * from std;
import * from std.dispatch;
import {Option, contains} from option;
import {sender} from context;

// A token either has an owner or does not exist: unset mapping entries
// read back as Option.None.
contract MiniNFT {
    nextId : uint256;
    owners : mapping(uint256 => Option<address>);
    approvals : mapping(uint256 => Option<address>);
    balances : mapping(address => uint256);

    constructor() {}

    function mint() public returns (uint256) {
        let id = nextId;
        nextId = nextId + uint256(1);
        let to = sender();
        owners[id] = Option.Some(to);
        balances[to] = balances[to] + uint256(1);
        return id;
    }

    function ownerOf(id: uint256) public returns (address) {
        match (owners[id]) {
            case Option.Some(owner) { return owner; }
            default {
                require(false, "no such token");
                return address(0);
            }
        }
    }

    function approve(to: address, id: uint256) public {
        require(sender() == ownerOf(id), "not the owner");
        approvals[id] = Option.Some(to);
    }

    function transfer(to: address, id: uint256) public {
        let owner = ownerOf(id);
        let from = sender();
        require(from == owner || contains(approvals[id], from), "not authorized");
        approvals[id] = Option.None;
        owners[id] = Option.Some(to);
        balances[owner] = balances[owner] - uint256(1);
        balances[to] = balances[to] + uint256(1);
    }

    function balanceOf(who: address) public returns (uint256) {
        return balances[who];
    }

    function exists(id: uint256) public returns (bool) {
        match (owners[id]) {
            case Option.Some(_) { return true; }
            default { return false; }
        }
    }
}
`,
      },
      {
        path: "option.sol",
        content: `// Candidate standard-library inventory: this module should disappear once
// std provides Option.
import * from std;
import * from std.Generic;
import * from std.StorageGeneric;

export { Option(*), checkedDiv, unwrapOr, contains };

enum Option<a> {
    None,
    Some(a)
}

// Division by zero yields Option.None instead of reverting.
function checkedDiv(a: uint256, b: uint256) returns (Option<uint256>) {
    if (b == uint256(0)) {
        return Option.None;
    }
    return Option.Some(a / b);
}

function unwrapOr<a>(option: Option<a>, orElse: a) returns (a) {
    match (option) {
        case Option.Some(value) { return value; }
        default { return orElse; }
    }
}

function contains<a>(option: Option<a>, value: a) returns (bool) where a: Eq {
    match (option) {
        case Option.Some(inner) { return inner == value; }
        default { return false; }
    }
}
`,
      },
      {
        path: "context.sol",
        content: `// Candidate standard-library inventory: this module should disappear once
// std provides transaction context helpers.
import * from std;
import {caller} from std.opcodes;

export { sender };

// msg.sender: the CALLER opcode lifted from word into address.
function sender() returns (address) {
    return address(caller());
}
`,
      },
    ],
  },
  {
    id: "composition",
    name: "Composition",
    description: "Three deployable vaults share one engine module: traits compose where Classic Solidity builds inheritance diamonds.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import * from std;
import * from std.dispatch;
import {settle, Direct, Signed, NoFee, FlatFee, BasisFee, Stacked} from engine;

// Three deployable vaults sharing the engine module, each binding its own
// context and fee choice. This file is the counterpart of an inheritance
// diamond's leaf contracts; note what is absent: override lists, super
// chains, and linearization order. The trade: each leaf repeats its two
// storage lines, because storage stays contract-scoped. The vaults track
// credits only; token custody is elided.

contract VaultDirect {
    balances : mapping(address => uint256);

    function deposit(amount: uint256) public {
        match (settle(Direct, NoFee, amount)) {
            case (who, credited) { balances[who] = balances[who] + credited; }
        }
    }

    function balanceOf(who: address) public returns (uint256) {
        return balances[who];
    }
}

// Gasless deposits: anyone may relay the call, and the credited account is
// recovered from a signature over the amount. Real code would also bind a
// nonce, the chain id, and the vault address into the digest to prevent
// replay.
contract VaultGasless {
    balances : mapping(address => uint256);
    collected : uint256;
    flatFee : uint256;

    constructor(fee: uint256) {
        flatFee = fee;
    }

    function depositFor(amount: uint256, v: uint256, r: bytes32, s: bytes32) public {
        let digest = bytes32(hash1(Num.toWord(amount)));
        match (settle(Signed(digest, v, r, s), FlatFee(flatFee), amount)) {
            case (who, credited) {
                balances[who] = balances[who] + credited;
                collected = collected + (amount - credited);
            }
        }
    }

    function balanceOf(who: address) public returns (uint256) {
        return balances[who];
    }

    function feesCollected() public returns (uint256) {
        return collected;
    }
}

// Stacked fees: a protocol fee in basis points, then a flat tip, ordered by
// the expression below.
contract VaultPremium {
    balances : mapping(address => uint256);

    function deposit(amount: uint256) public {
        let policy = Stacked(BasisFee(uint256(30)), FlatFee(uint256(2)));
        match (settle(Direct, policy, amount)) {
            case (who, credited) { balances[who] = balances[who] + credited; }
        }
    }

    function balanceOf(who: address) public returns (uint256) {
        return balances[who];
    }
}
`,
      },
      {
        path: "engine.sol",
        content: `// The composition machinery shared by every vault in main.sol. In Classic
// Solidity this role is played by base contracts and virtual functions; the
// crossings then need override(...) lists. Traits have one impl per type,
// so there is nothing to disambiguate.
import * from std;
import {sender} from context;

export {
    TxnContext,
    FeePolicy,
    settle,
    Direct(*),
    Signed(*),
    NoFee(*),
    FlatFee(*),
    BasisFee(*),
    Stacked(*)
};

// Axis one: where does the acting address come from?
trait TxnContext<c> {
    function originator(ctx: c) returns (address);
}

// A direct call: the transaction caller acts for themselves.
enum Direct { Direct }

impl TxnContext<Direct> {
    function originator(ctx: Direct) returns (address) {
        return sender();
    }
}

// A relayed call: the acting address is recovered from a signature over
// the digest the relayer hands in alongside it.
enum Signed { Signed(bytes32, uint256, bytes32, bytes32) }

impl TxnContext<Signed> {
    function originator(ctx: Signed) returns (address) {
        match (ctx) {
            case Signed(digest, v, r, s) {
                // std's ecrecover reverts on malleable, failed, or
                // zero-address recovery, so this can never return a bogus
                // signer. The invariant lives in one place.
                return ecrecover(digest, v, r, s);
            }
        }
    }
}

// Axis two: how much of a deposit is credited?
trait FeePolicy<f> {
    function afterFee(policy: f, amount: uint256) returns (uint256);
}

enum NoFee { NoFee }

impl FeePolicy<NoFee> {
    function afterFee(policy: NoFee, amount: uint256) returns (uint256) {
        return amount;
    }
}

enum FlatFee { FlatFee(uint256) }

impl FeePolicy<FlatFee> {
    function afterFee(policy: FlatFee, amount: uint256) returns (uint256) {
        match (policy) {
            case FlatFee(fee) {
                if (amount > fee) { return amount - fee; }
                return uint256(0);
            }
        }
    }
}

// A percentage fee in basis points (parts per ten thousand).
enum BasisFee { BasisFee(uint256) }

impl FeePolicy<BasisFee> {
    function afterFee(policy: BasisFee, amount: uint256) returns (uint256) {
        match (policy) {
            case BasisFee(bps) {
                // std uint256 arithmetic is unchecked today: the multiply
                // wraps for amounts above 2^256 / bps.
                return amount - amount * bps / uint256(10000);
            }
        }
    }
}

// Policies compose as values, applied left to right. The order is the
// expression written at the use site, not the C3 linearization of an
// inheritance list.
enum Stacked<f, g> { Stacked(f, g) }

impl<f, g> FeePolicy<Stacked<f, g>> where f: FeePolicy, g: FeePolicy {
    function afterFee(policy: Stacked<f, g>, amount: uint256) returns (uint256) {
        match (policy) {
            case Stacked(first, second) {
                return FeePolicy.afterFee(second, FeePolicy.afterFee(first, amount));
            }
        }
    }
}

// The engine, written once for every context and fee policy: who gets
// credited with how much. Storage stays with each contract.
function settle<c, f>(ctx: c, policy: f, amount: uint256) returns ((address, uint256))
    where c: TxnContext, f: FeePolicy
{
    return (TxnContext.originator(ctx), FeePolicy.afterFee(policy, amount));
}
`,
      },
      {
        path: "context.sol",
        content: `// Candidate standard-library inventory: this module should disappear once
// std provides transaction context helpers.
import * from std;
import {caller} from std.opcodes;

export { sender };

// msg.sender: the CALLER opcode lifted from word into address.
function sender() returns (address) {
    return address(caller());
}
`,
      },
    ],
  },
  {
    id: "comptime",
    name: "Comptime",
    description: "Evaluates a typed computation during specialization and embeds its result.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import * from std;
import * from std.dispatch;

// Evaluated during specialization: the deployed code contains only the
// result, not the computation.
function double(comptime value: uint256) returns (comptime<uint256>) {
    return value + value;
}

contract Answer {
    function answer() public returns (uint256) {
        let result: comptime<uint256> = double(uint256(21));
        return result;
    }
}
`,
      },
    ],
  },
];

export const defaultExample = examples[0];

export function findExample(id: string): PlaygroundExample | undefined {
  return examples.find((example) => example.id === id);
}

export function getExample(id: string): PlaygroundExample {
  return findExample(id) ?? defaultExample;
}
