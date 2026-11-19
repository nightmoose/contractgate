import assert from "node:assert/strict";
import test from "node:test";
import { localUpgradeMessage } from "./impact-upgrade";

test("upgrade copy on staging stays on the current host", () => {
  const message =
    "Downstream impact analysis is included with Pro + Impact and Enterprise. Upgrade at https://app.datacontractgate.com/pricing";
  assert.equal(
    localUpgradeMessage(message),
    "Downstream impact analysis is included with Pro + Impact and Enterprise. Upgrade at /pricing",
  );
  assert.equal(localUpgradeMessage("Nothing to rewrite"), "Nothing to rewrite");
  assert.equal(
    localUpgradeMessage("Upgrade at https://app.datacontractgate.com/pricing?plan=impact"),
    "Upgrade at /pricing",
  );
});
