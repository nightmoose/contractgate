import assert from "node:assert/strict";
import test from "node:test";
import { synthToCsv } from "./synth-csv";
import type { SynthResponse } from "./api";

const base = {
  contract: { name: "t", version: "1" },
  source: {},
  seed: 1,
  anchor_time: "2026-01-01T00:00:00Z",
  skipped_rules: [],
};

test("valid rows flatten like the CLI", () => {
  const res = {
    ...base,
    mode: "valid",
    row_count: 2,
    rows: [{ id: "a", ship: { country: "US" }, tags: ["x"], n: 2, e: {} }, { id: "b,c" }],
  } as SynthResponse;
  assert.deepEqual(synthToCsv(res).split("\n"), [
    "id,ship.country,tags,n,e",
    'a,US,"[""x""]",2,{}',
    '"b,c",,,,',
    "",
  ]);
});

test("invalid rows carry the broken rule", () => {
  const res = {
    ...base,
    mode: "invalid",
    row_count: 1,
    rows: [
      {
        record: { id: 5 },
        broken_rule: { field: "id", rule: "type", violation_kind: "type_mismatch", message: "m" },
      },
    ],
  } as SynthResponse;
  assert.equal(synthToCsv(res), "id,_broken_field,_broken_rule\n5,id,type\n");
});
