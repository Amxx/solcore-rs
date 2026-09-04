import contractOutputHello from "./contract-output/Hello.sol?raw";
import stdUsageCalculator from "./std-usage/Calculator.sol?raw";
import traitLightSwitch from "./trait/LightSwitch.sol?raw";
import genericsRegistry from "./generics/Registry.sol?raw";
import optionSplitter from "./option/Splitter.sol?raw";
import optionOption from "./option/option.sol?raw";
import patternMatchingEscrow from "./pattern-matching/Escrow.sol?raw";
import miniNftMiniNFT from "./mini-nft/MiniNFT.sol?raw";
import miniNftOption from "./mini-nft/option.sol?raw";
import miniNftContext from "./mini-nft/context.sol?raw";
import compositionVaults from "./composition/Vaults.sol?raw";
import compositionEngine from "./composition/engine.sol?raw";
import compositionContext from "./composition/context.sol?raw";
import invariantsAmm from "./invariants/Amm.sol?raw";
import invariantsPool from "./invariants/pool.sol?raw";
import comptimeAnswer from "./comptime/Answer.sol?raw";

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
    entry: "Hello.sol",
    files: [
      { path: "Hello.sol", content: contractOutputHello },
    ],
  },
  {
    id: "std-usage",
    name: "Std usage",
    description: "Calls a standard-library trait method directly; the + operator desugars to the same call.",
    entry: "Calculator.sol",
    files: [
      { path: "Calculator.sol", content: stdUsageCalculator },
    ],
  },
  {
    id: "trait",
    name: "Trait",
    description: "A trait implemented for a custom enum drives a stored on-chain state machine.",
    entry: "LightSwitch.sol",
    files: [
      { path: "LightSwitch.sol", content: traitLightSwitch },
    ],
  },
  {
    id: "generics",
    name: "Generics",
    description: "A where-constrained generic max works for a user-defined Version type via its Ord impl.",
    entry: "Registry.sol",
    files: [
      { path: "Registry.sol", content: genericsRegistry },
    ],
  },
  {
    id: "option",
    name: "Option",
    description: "A reusable Option module and a splitter contract: checked division returns an Option instead of reverting.",
    entry: "Splitter.sol",
    files: [
      { path: "Splitter.sol", content: optionSplitter },
      { path: "option.sol", content: optionOption },
    ],
  },
  {
    id: "pattern-matching",
    name: "Pattern matching",
    description: "An escrow whose lifecycle is an enum stored in a contract field, driven by match.",
    entry: "Escrow.sol",
    files: [
      { path: "Escrow.sol", content: patternMatchingEscrow },
    ],
  },
  {
    id: "mini-nft",
    name: "Mini NFT",
    description: "An NFT with typed ownership: tokens either have an owner or do not exist, with no zero-address sentinels.",
    entry: "MiniNFT.sol",
    files: [
      { path: "MiniNFT.sol", content: miniNftMiniNFT },
      { path: "option.sol", content: miniNftOption },
      { path: "context.sol", content: miniNftContext },
    ],
  },
  {
    id: "composition",
    name: "Composition",
    description: "Three deployable vaults share one engine module: traits compose where Classic Solidity builds inheritance diamonds.",
    entry: "Vaults.sol",
    files: [
      { path: "Vaults.sol", content: compositionVaults },
      { path: "engine.sol", content: compositionEngine },
      { path: "context.sol", content: compositionContext },
    ],
  },
  {
    id: "invariants",
    name: "Invariants",
    description: "A constant-product pool as an opaque type: it can only be created or changed through module operations that preserve the invariant.",
    entry: "Amm.sol",
    files: [
      { path: "Amm.sol", content: invariantsAmm },
      { path: "pool.sol", content: invariantsPool },
    ],
  },
  {
    id: "comptime",
    name: "Comptime",
    description: "Evaluates a typed computation during specialization and embeds its result.",
    entry: "Answer.sol",
    files: [
      { path: "Answer.sol", content: comptimeAnswer },
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
