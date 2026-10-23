# Webhook quarantine gate

A FastAPI receiver that validates an inbound webhook with **DataContractGate**
before anything downstream sees it.

`POST /v1/ingest/{contract_id}` runs the contract (types, `enum`, `pattern`,
`compliance_mode`, and PII `transform` rules). The request sends
`Idempotency-Key` so a provider retry does not double-process. On pass, the
receiver forwards `transformed_event` — the PII-masked payload — and not the
raw body. On failure, DataContractGate has already quarantined the event.
Replay it later with `POST /quarantine/replay` after the producer or the
contract is fixed. This process does not store a second copy.

```bash
python app.py --self-check

export CONTRACTGATE_API_KEY=cg_live_...
export CONTRACTGATE_CONTRACT_ID=<deployed uuid>
export DOWNSTREAM_URL=https://example.internal/billing
pip install -r requirements.txt
uvicorn app:app --host 0.0.0.0 --port 8080
```

Contract: [`contract.yaml`](contract.yaml). Deploy it, then point
`CONTRACTGATE_CONTRACT_ID` at that version's contract id.

The idempotency key is the first of `svix-id`, `x-hookdeck-eventid`,
`x-request-id`, the JSON `id`, or a hash of the body. Stripe's event id in
`id` is the usual key for that provider.

A failed ingest returns **422** to the provider only when you want the provider
to retry. The default here is **202** with `quarantine_id`: the event is
safely stored, and a retry would only enqueue a duplicate. Change
`reject_status` in `handle_webhook` if the provider should retry on its own.
