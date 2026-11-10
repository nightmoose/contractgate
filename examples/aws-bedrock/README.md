# ContractGate + Amazon Bedrock: check model output before you forward it

A model on Amazon Bedrock turns short business notes into JSON. ContractGate checks
each JSON object against a YAML contract. Each note ends one of three ways:

- **PASS**: the first answer meets the contract and is forwarded.
- **REPAIR**: the check fails, a second Bedrock model gets the violations and the
  contract, rewrites the JSON, and the re-check passes. The repaired record is forwarded.
- **QUARANTINE**: the re-check still fails. The record is held back with its
  violation report and nothing is forwarded.

```
note ──► Bedrock extractor (Nova Lite) ──► ContractGate check ──► PASS
                                              │ violations
                                              ▼
                               Bedrock repairer (Nova Pro) ──► re-check ──► REPAIR
                                                                  │ still failing
                                                                  ▼
                                                              QUARANTINE
```

This is the egress boundary: model output is checked before it goes to the next
service. The same contract format and engine also run at ingest and between stages.

## Run it

You need AWS credentials that can call `bedrock:Converse` in a region where Amazon
Nova Lite and Nova Pro are available (us-east-1 by default). You don't need a ContractGate account.

```bash
cd examples/aws-bedrock
python -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt
python bedrock_gate.py samples/            # add --json report.json for a machine-readable report
```

Options:

| Flag / env | Default | What it does |
|---|---|---|
| `BEDROCK_EXTRACTOR_MODEL` | `amazon.nova-lite-v1:0` | First-pass extractor |
| `BEDROCK_REPAIRER_MODEL` | `amazon.nova-pro-v1:0` | Repairer, called only after a violation |
| `--region` | `$AWS_REGION` or `us-east-1` | Bedrock region |
| `--hosted` | off | Check with the hosted gateway (`POST /egress/{contract}`, `dry_run`) instead of in process. Needs `CONTRACTGATE_API_KEY`, and the two contracts deployed to your org under the same names. |
| `--slow N` | 0 | Pause N seconds between steps, which helps when you screen-record |

By default the check runs in process with the `contractgate` Python package
(`contractgate.validator.validate`), so the run makes no ContractGate network calls.

## What the samples show (recorded run, 2026-11-10, us-east-1)

| Sample | First answer | Outcome |
|---|---|---|
| `01_invoice_pass` | Matches the contract | **PASS** |
| `02_invoice_repair` | `"Bill A-9"` fails the id pattern, `"€"` and `"past due, second reminder sent"` fail their enums | **REPAIR** to `A-9` / `EUR` / `overdue` |
| `03_ticket_pass` | Matches the contract | **PASS** |
| `04_ticket_quarantine` | No ticket number in the note. The repairer was told never to invent one and still made up `"login_issue_password_reset"`. The `ticket_id` pattern rejects it | **QUARANTINE** |

Sample 04 is the point of the demo. The repaired answer is schema-valid JSON with a
non-empty string id, and it is still wrong. The contract's `pattern` is what keeps
that made-up id out of the downstream system.

Full output: [`sample-run.txt`](sample-run.txt) and [`sample-run.json`](sample-run.json).
Check time was 16 to 64 µs per object in process. Bedrock calls took 0.35 to 0.9 s.
A full run of the four samples is about 1,300 tokens, or roughly $0.001 at on-demand Nova prices.

## Files

- `bedrock_gate.py`: the demo (Bedrock Converse API with boto3, then the ContractGate check)
- `contracts/invoice_v1.yaml`, `contracts/ticket_v1.yaml`: the contracts (types, required fields, enums, ranges, lengths, id patterns)
- `samples/*.txt`: the notes. The file name selects the contract (`invoice` or `ticket`).
