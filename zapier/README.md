# ContractGate for Zapier

The step that checks a record Zapier just pulled, before the next step writes it.

**Validate Record** posts that one object to `POST /v1/ingest/{contract_id}`.

- A failure is quarantined. The Zap stops. Replay happens in ContractGate, not in Zapier's task history. A halted task is not replayed by Zapier, on purpose: replaying it would send the same bad record again.
- A pass continues. The next step writes **Payload**, which is the record after the contract's PII transforms. On a failure, Payload is null.
- **New Quarantined Record** polls `GET /quarantine` (newest 100) for Zaps that should hear about rejects from other producers too.

Auth is an API key (`cg_live_…`) from [Account → API keys](https://app.datacontractgate.com/account). Set Gateway URL only if you self-host.

How to build the Zap, including the Code step that reassembles a trigger's fields into one JSON object: [`docs/zapier.md`](../docs/zapier.md).

## Put it in the Zap editor

From this directory, with [a Zapier deploy key](https://developer.zapier.com/partner-settings/deploy-keys/):

Run these from this directory. If your shell is already here, do not `cd zapier` again. `npx zapier-platform` is the binary from the `zapier-platform-cli` dev dependency installed by `npm install` — there is no npm package named `zapier-platform`.

```bash
npm install
npx zapier-platform login --sso
npx zapier-platform register "ContractGate"
npx zapier-platform push
```

`register` writes `.zapierapprc`. Commit that file so the next push updates the same integration instead of creating a second one. The integration is private until you submit the version for listing in the Zapier developer UI. Invite yourself, build one Zap, then submit.

Node 22 is what Zapier runs. Tests here run on Node 18+.

```bash
npm test
```
