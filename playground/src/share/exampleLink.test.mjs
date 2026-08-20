import assert from "node:assert/strict";
import test from "node:test";

import { buildExampleLink, readSharedExampleId, stripExampleParam } from "./exampleLink.js";

test("reads the example id from a query string", () => {
  assert.equal(readSharedExampleId("?example=trait"), "trait");
  assert.equal(readSharedExampleId("?foo=1&example=mini-nft"), "mini-nft");
});

test("ignores a missing, empty, or blank example id", () => {
  assert.equal(readSharedExampleId(""), null);
  assert.equal(readSharedExampleId("?foo=1"), null);
  assert.equal(readSharedExampleId("?example="), null);
  assert.equal(readSharedExampleId("?example=%20%20"), null);
});

test("builds a link that keeps the deployment subpath", () => {
  assert.equal(
    buildExampleLink("https://example.org/solcore-rs/", "hello"),
    "https://example.org/solcore-rs/?example=hello",
  );
});

test("building a link replaces an existing example id and drops the hash", () => {
  assert.equal(
    buildExampleLink("https://example.org/?example=hello#frag", "generics"),
    "https://example.org/?example=generics",
  );
});

test("stripping removes only the example parameter", () => {
  assert.equal(stripExampleParam("https://example.org/?example=hello"), "https://example.org/");
  assert.equal(
    stripExampleParam("https://example.org/?theme=dark&example=hello"),
    "https://example.org/?theme=dark",
  );
});

test("stripping leaves an href without the parameter untouched", () => {
  assert.equal(stripExampleParam("https://example.org/app"), "https://example.org/app");
});
