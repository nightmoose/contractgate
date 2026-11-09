import assert from "node:assert/strict";
import test from "node:test";
import { boundaryLabel } from "./contract-family";

test("boundary labels name the pipeline edge", () => {
  assert.equal(boundaryLabel("ingest"), "Ingest");
  assert.equal(boundaryLabel("stage"), "Between stages");
  assert.equal(boundaryLabel("egress"), "Egress");
  assert.equal(boundaryLabel("mart"), "mart");
});
