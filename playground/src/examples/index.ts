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
