import assert from "node:assert/strict";
import test from "node:test";
import { synthErrorMessage } from "./synth-errors";

test("server rows limit reads as a form hint", () => {
  assert.equal(
    synthErrorMessage({
      error: "invalid_rows",
      field: null,
      message: "rows must be between 1 and 10000, got 10001",
      rule: "rows",
      status: 400,
    }),
    "Rows must be a whole number from 1 to 10000 (you asked for 10001).",
  );
  assert.equal(
    synthErrorMessage({ error: "invalid_rows", message: "" }),
    "Rows must be a whole number from 1 to 10000.",
  );
});

test("unsatisfiable rule names rule and field", () => {
  assert.equal(
    synthErrorMessage({
      error: "unsatisfiable_rule",
      rule: "min_length",
      field: "c",
      message: "field 'c' has min_length 5 above max_length 2",
    }),
    "No rows can satisfy the min_length rule on 'c'. Field 'c' has min_length 5 above max_length 2.",
  );
});

test("no_envelope and invalid_contract", () => {
  assert.match(synthErrorMessage({ error: "no_envelope", message: "x" }) ?? "", /declares no envelope/);
  assert.equal(
    synthErrorMessage({ error: "invalid_contract", message: "bad yaml" }),
    "The contract could not be compiled: bad yaml",
  );
});

test("other bodies are not synth errors", () => {
  assert.equal(synthErrorMessage({ error: "Unauthorized" }), null);
  assert.equal(synthErrorMessage("nope"), null);
  assert.equal(synthErrorMessage(null), null);
});
