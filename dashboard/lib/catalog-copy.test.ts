import assert from "node:assert/strict";
import test from "node:test";
import { catalogCardLine, catalogHeaderAside } from "./catalog-copy";
import { catalogSampleEvent } from "./playground-prefill";

test("GA4 header names Google, not the product, as the unaffiliated company", () => {
  const aside = catalogHeaderAside("ga4-bigquery-events", "Google Analytics 4");
  assert.match(aside, /compatible with GA4/);
  assert.match(aside, /not affiliated with Google/);
  assert.doesNotMatch(aside, /not affiliated with Google Analytics 4/);
});

test("generic CSV copy does not affiliate DataContractGate with itself", () => {
  const aside = catalogHeaderAside("generic-sheets-transactions-csv", "Spreadsheet CSV");
  const card = catalogCardLine("generic-sheets-transactions-csv", "Spreadsheet CSV");
  for (const line of [aside, card]) {
    assert.doesNotMatch(line, /DataContractGate/);
    assert.doesNotMatch(line, /not affiliated/);
    assert.doesNotMatch(line, /compatible with/);
  }
  assert.match(aside, /spreadsheet \/ CSV transactions export/);
  assert.match(card, /spreadsheet \/ CSV transactions export/);
});

test("vendor contracts keep the shared affiliation line", () => {
  assert.equal(
    catalogHeaderAside("stripe-charges", "Stripe"),
    "(compatible with Stripe’s documented schema; not affiliated with Stripe)"
  );
  assert.equal(catalogCardLine("acs-5year-2023-county", "US Census Bureau"), "compatible with US Census Bureau");
});

test("catalog sample event is the first published object", () => {
  const event = catalogSampleEvent([
    { id: "ch_example" },
    { id: "py_example" },
  ]);
  assert.equal(event, '{\n  "id": "ch_example"\n}');
  assert.equal(catalogSampleEvent(null), null);
  assert.equal(catalogSampleEvent([]), null);
  assert.equal(catalogSampleEvent(["nope", null]), null);
});
