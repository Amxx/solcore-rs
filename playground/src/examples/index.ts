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
    description: "Defines a trait, implements it for a custom enum, and calls its method.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `trait Toggle<a> {
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

function main() returns (Switch) {
    return Toggle.toggle(Switch.Off);
}
`,
      },
    ],
  },
  {
    id: "generics",
    name: "Generics",
    description: "Uses a generic pair type and a generic function to select its second value.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `enum Pair<a, b> {
    Pair(a, b)
}

function second<a, b>(pair: Pair<a, b>) returns (b) {
    match (pair) {
        case Pair(_, value) { return value; }
    }
}

function main() returns (word) {
    return second(Pair(true, 42));
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

export { Option(*), checkedDiv, unwrapOr, contains };
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

export { Option(*), checkedDiv, unwrapOr, contains };
`,
      },
      {
        path: "context.sol",
        content: `// Candidate standard-library inventory: this module should disappear once
// std provides transaction context helpers.
import * from std;
import {caller} from std.opcodes;

// msg.sender: the CALLER opcode lifted from word into address.
function sender() returns (address) {
    return address(caller());
}

export { sender };
`,
      },
    ],
  },
  {
    id: "lambda",
    name: "Lambda",
    description: "Builds a lambda that captures a value from its enclosing function.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `function makeAdder(value: word) returns (function(word) returns (word)) {
    return lam (other: word) -> word {
        let result: word;
        assembly {
            result := add(value, other)
        }
        return result;
    };
}

function main() returns (word) {
    let addTen = makeAdder(10);
    return addTen(32);
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

function double(comptime value: word) returns (comptime<word>) {
    return value + value;
}

function main() returns (word) {
    let answer: comptime<word> = double(21);
    return answer;
}
`,
      },
    ],
  },
  {
    id: "multi-file",
    name: "Multi-file",
    description: "Imports a sibling module and calls an exported function.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import {double} from math;

function main() returns (word) {
    return double(21);
}
`,
      },
      {
        path: "math.sol",
        content: `function double(x: word) returns (word) {
    let res: word;
    assembly {
        res := add(x, x)
    }
    return res;
}

export { double };
`,
      },
    ],
  },
];

export const defaultExample = examples[0];

export function getExample(id: string): PlaygroundExample {
  return examples.find((example) => example.id === id) ?? defaultExample;
}
