# ContractGate on Amazon Kinesis Data Streams: ingest, stage, and egress checks

Stripe-shaped charge records go through a Kinesis pipeline. One contract format and
one engine check them at three boundaries. The contracts are the
[Stripe contract chain](../contract-chain/stripe/), the same family as the public catalog.

```
producer ──► nmb-cg-raw (2 shards)
               │
               ▼
        [ingest gate]  stripe_charges_raw
               │ pass
               ▼
          transform (cents → decimal, upper-case currency, epoch → ISO-8601, flatten customer)
               │
               ▼
        [stage gate]   stripe_charges_staged
               │ pass
               ▼
         nmb-cg-staged (2 shards) ──► roll up to partner payout rows
                                            │
                                            ▼
                                     [egress gate]  partner_payout_feed
                                            │ pass
                                            ▼
                                      nmb-cg-payouts

  every gate's failures ──► nmb-cg-quarantine  (record + boundary + violation report)
```

## Run it

You need AWS credentials with Kinesis create, put, get, and delete permissions. Four provisioned
streams of 1 to 2 shards each cost about $0.10 an hour together, plus PUT payload units.
Run `down` when you're done.

```bash
cd examples/aws-kinesis
python -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt
python pipeline.py up                                  # creates nmb-cg-{raw,staged,quarantine,payouts}
python pipeline.py run --records 100000 --rate 1900    # produce, gate, stage, egress, print the numbers
python pipeline.py down                                # deletes the streams
```

`STREAM_PREFIX` changes the `nmb-cg` prefix. `TRANSFORM_BUG_RATE` (default `0.02`) sets how often the
transform's known bug fires (it writes epoch milliseconds into `charged_at`), so the stage gate has a real regression to catch.

`pipeline.py` runs the gates as a polling consumer (`GetRecords` per shard), so you can see every
number from one terminal. [`lambda_function.py`](lambda_function.py) is the same ingest and stage
logic as a Lambda handler for a Kinesis event source mapping. `python lambda_function.py` tests it locally
with the chain's pass and fail samples (`DRY_RUN`, no AWS calls). The `contractgate` package is pure
Python, so the Lambda bundle needs no native build.

## Measured runs (us-east-1, 2026-11-10)

The consumer ran on a 4 vCPU box in us-east-2. It used the in-process `contractgate` validator.

| Run | Records | Producer rate | Gated throughput | Kinesis arrival → gated, p50 / p95 / p99 | Check time per record, p50 / p99 |
|---|---|---|---|---|---|
| [20k](runs/2026-11-10-20k-1500rps.json) | 20,000 | 1,500/s | 1,500/s | 151 / 403 / 519 ms | 8.7 / 25.1 µs |
| [100k](runs/2026-11-10-100k-1900rps.json) | 100,000 | 1,900/s | 1,882/s | 251 / 452 / 529 ms | 8.4 / 26.2 µs |
| [100k, staged stream at 1 shard](runs/2026-11-10-100k-1900rps-staged-1shard.json) | 100,000 | 1,900/s | 1,279/s | 6.3 / 21.8 / 23.3 s | 8.1 / 30.1 µs |

What the 100k run caught:

| Boundary | Contract | Held back | Why |
|---|---|---|---|
| Ingest | `stripe_charges_raw` | 5,018 (5.0%) | negative `amount` (1,038), missing `created` (1,030), `txn_` id prefix (989), upper-case `currency` (984), `status: complete` (977) |
| Stage | `stripe_charges_staged` | 4,910 (4.9%) | guest checkouts with no `customer`, which is optional at ingest and required once staged (3,085), and the transform bug writing epoch ms into `charged_at` (~1,800) |
| Egress | `partner_payout_feed` | 12 of 48 payout rows | JPY days where every charge failed, so the payout came to 0.00, which is below `min: 0.01`. The row was held instead of sending a partner a zero payout |

90,072 records reached the staged stream. 36 payout rows went out.

Notes:

- The slow third run is a Kinesis limit, not the gate. A shard takes 1,000 records/s of writes, and about
  1,700/s were passing into a 1-shard staged stream. With 2 shards the lag went back to sub-second.
  Size each output stream for the pass rate, not the input rate.
- **Known gap found by this demo:** the producer also sent 967 records with
  `receipt_email: "billing at example dot com"`. The contract says `format: email`, and the Rust engine
  (`src/validation.rs`) enforces it. The Python package's in-process validator (`contractgate` 0.4.1)
  does not check `format`, so those records passed ingest. Use the hosted gateway, or the Rust engine,
  when you depend on `format`, until the Python validator reaches parity.
