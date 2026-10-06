# ContractGate on Zapier

Zapier pulls a lead, an order, a row, a form submission out of a system that will never grow a producer. **Validate Record** is the step that checks that object against a contract before the next step writes it.

A failure is quarantined and the Zap stops. A pass continues, and the next step writes the **Payload** ContractGate returned. Declared PII is already masked.

The integration source is [`zapier/`](../zapier/). It is not in the public Zapier directory until that version is submitted and approved. Pushing it puts the action in your Zap editor the same day.

## 1. Contract and key

1. Sign in at <https://app.datacontractgate.com> and create a key under **Account → API keys**. It starts with `cg_live_`.
2. Deploy a stable contract for the record Zapier pulls. The integration playbook does this from samples: <https://app.datacontractgate.com/llm-integration.md>.

The action calls `POST /v1/ingest/{contract_id}` with that key. The contract dropdown calls `GET /contracts`.

## 2. Put the action in the Zap editor

From the `zapier` directory of a checkout. If your shell prompt already ends in `zapier`, do not `cd zapier` again.

```bash
npm install
npx zapier-platform login --sso
npx zapier-platform register "ContractGate"
npx zapier-platform push
```

`npx zapier-platform` is the CLI binary from the `zapier-platform-cli` dev dependency. It is not a separate npm package named `zapier-platform`. `npm install` in this directory is what puts that binary on `npx`.

Log in with a [deploy key](https://developer.zapier.com/partner-settings/deploy-keys/) if the account uses Google, Apple, or SSO (`login --sso`).

`register` writes `zapier/.zapierapprc`. Commit it. The next `push` updates this integration instead of creating another one.

The version is private. In the Zap editor, search for ContractGate. After one real Zap works, submit that version for the public listing from the Zapier developer UI.

Self-hosted gateway: set **Gateway URL** on the connection. Cloud users leave `https://app.datacontractgate.com`.

## 3. Build the Zap

1. Trigger: the app Zapier is pulling from (Salesforce, a form, a sheet, Shopify, whatever has the record).
2. Most triggers hand you separate fields, not one JSON object. Add **Code by Zapier** → Run Javascript:

```javascript
output = [{ record: JSON.stringify(inputData) }];
```

3. **ContractGate → Validate Record**
   - Contract: pick the one you deployed.
   - Record: map `record` from the Code step.
   - Leave **Stop the Zap when the record fails** on.
   - Leave **Dry run** off once you have seen one pass and one fail. Dry run writes nothing.
   - Idempotency Key: map a stable source id (lead id, order id) so a Zap retry does not quarantine the same record twice inside 24 hours.
4. Next step: the write (sheet, warehouse, the next app). Map fields from **Payload**, not from the original trigger.

Turn **Stop the Zap when the record fails** off only when a Path or Filter should handle the reject. Filter on **Passed** is true. Payload is null when the record failed, so a later step cannot write the bad object by mapping Payload.

**New Quarantined Record** polls `GET /quarantine` (newest 100) if something else needs to hear about rejects — a Slack message, a ticket — including rejects that did not come from this Zap.

## What a reject does

ContractGate stores the record and the violations. The Zap task is halted, not failed, so a string of bad records will not turn the Zap off. Zapier will not replay a halted task. Fix the source record or the contract, then replay from ContractGate. That is `POST /quarantine/replay`, or the Quarantine tab.

A dry-run reject halts (when the stop switch is on) and quarantines nothing.

## What you can map

| Field | On a pass | On a fail, stop switch off |
|---|---|---|
| Passed | true | false |
| Payload | The record the next step should write | null |
| Violations | empty | One line per violation: `field: message` |
| Quarantine ID | empty | The quarantine row, unless this was a dry run |
| Contract Version | The version that ran | The version that ran |
