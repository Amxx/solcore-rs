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
    name: "Contract output",
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
    id: "hello",
    name: "Hello",
    description: "A minimal function returning a word literal.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `function main() returns (word) {
    return 42;
}
`,
      },
    ],
  },
  {
    id: "std-usage",
    name: "Std usage",
    description: "Imports a helper from the embedded standard library.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import {addWord} from std;

function main() returns (word) {
    return addWord(1, 2);
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
    description: "A generic optional value: checked division returns an Option instead of reverting.",
    entry: "main.sol",
    files: [
      {
        path: "main.sol",
        content: `import * from std;

enum Option<a> {
    None,
    Some(a)
}

// Division by zero yields Option.None instead of reverting.
function checkedDiv(a: word, b: word) returns (Option<word>) {
    if (b == 0) {
        return Option.None;
    }
    return Option.Some(a / b);
}

function unwrapOr(option: Option<word>, orElse: word) returns (word) {
    match (option) {
        case Option.Some(value) { return value; }
        default { return orElse; }
    }
}

function main() returns (word) {
    // 84 / 2 succeeds with Some(42); 84 / 0 would fall back to 0.
    return unwrapOr(checkedDiv(84, 2), 0);
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
