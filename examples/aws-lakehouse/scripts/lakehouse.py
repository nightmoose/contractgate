#!/usr/bin/env python3
"""P8 lakehouse data contracts: S3 + Iceberg + Glue Catalog + Athena.

Intended to run in GitHub Actions as nmb-terraform-deploy (OIDC), because
the NightMoose bot box VPC endpoint policy denies s3:CreateBucket.

Commands:
  python lakehouse.py run --records 10000   # create, load, query, write results
  python lakehouse.py down                  # tear down everything this stack creates

Never prints credentials. Does not touch ContractGate prod or Fly.
"""
from __future__ import annotations

import argparse
import json
import os
import random
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

import boto3
from botocore.exceptions import ClientError
from contractgate import Contract
from contractgate.validator import validate

REGION = os.environ.get("AWS_REGION") or os.environ.get("AWS_DEFAULT_REGION") or "us-east-1"
ACCOUNT = os.environ.get("AWS_ACCOUNT_ID", "065614567889")
BUCKET = os.environ.get("LAKEHOUSE_BUCKET", f"nmb-lakehouse-{ACCOUNT}")
DB = os.environ.get("LAKEHOUSE_DB", "nmb_lakehouse")
WORKGROUP = os.environ.get("LAKEHOUSE_WORKGROUP", "nmb-lakehouse")
CONTRACT_NAME = "stripe_charges_raw"
HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
CONTRACT_PATH = ROOT / "contracts" / f"{CONTRACT_NAME}.yaml"
DEFAULT_RESULTS = Path("results/aws/lakehouse")

TAGS = [
    {"Key": "ManagedBy", "Value": "github-actions"},
    {"Key": "Environment", "Value": "sandbox"},
    {"Key": "Owner", "Value": "nightmoose"},
    {"Key": "nmb:stack", "Value": "lakehouse"},
    {"Key": "nmb:cost-alloc", "Value": "aws-learning-plan"},
    {"Key": "nmb:do-not-prod", "Value": "true"},
    {"Key": "nmb:project", "Value": "P8"},
]

RAW_DEFECTS = [
    ("negative_amount", lambda c: c.update(amount=-abs(c["amount"]))),
    ("uppercase_currency", lambda c: c.update(currency=c["currency"].upper())),
    ("unknown_status", lambda c: c.update(status="complete")),
    ("bad_id_prefix", lambda c: c.update(id="txn_" + c["id"][3:])),
    ("bad_email", lambda c: c.update(receipt_email="billing at example dot com")),
    ("missing_created", lambda c: c.pop("created", None)),
]

CHARGE_COLUMNS = [
    ("id", "string"),
    ("object", "string"),
    ("amount", "int"),
    ("currency", "string"),
    ("customer", "string"),
    ("description", "string"),
    ("paid", "boolean"),
    ("refunded", "boolean"),
    ("captured", "boolean"),
    ("status", "string"),
    ("created", "bigint"),
    ("livemode", "boolean"),
    ("receipt_email", "string"),
    ("payment_intent", "string"),
]


def log(msg: str) -> None:
    print(msg, flush=True)


def clients():
    s = boto3.session.Session(region_name=REGION)
    return {
        "s3": s.client("s3"),
        "glue": s.client("glue"),
        "athena": s.client("athena"),
        "sts": s.client("sts"),
    }


def identity(c):
    ident = c["sts"].get_caller_identity()
    # Arn only — never print keys or the full response blob.
    return {"Account": ident["Account"], "Arn": ident["Arn"]}


def compiled_contract():
    return Contract.from_yaml(CONTRACT_PATH.read_text()).compile()


def make_charge(i: int, rng: random.Random, t0: int) -> tuple[dict, str]:
    cur = rng.choices(["usd", "eur", "gbp", "jpy"], [80, 12, 7, 1])[0]
    status = rng.choices(["succeeded", "pending", "failed"], [90, 5, 5])[0]
    c = {
        "id": f"ch_3Qdemo{i:08d}",
        "object": "charge",
        "amount": rng.randint(199, 49999),
        "currency": cur,
        "customer": f"cus_Demo{rng.randint(1, 400):04d}",
        "description": f"Demo invoice INV-DEMO-{i}",
        "paid": status == "succeeded",
        "refunded": rng.random() < 0.03,
        "captured": status == "succeeded",
        "status": status,
        "created": t0 - rng.randint(0, 3 * 86400),
        "livemode": False,
        "receipt_email": f"billing-{i % 997}@example.com",
        "payment_intent": f"pi_3Qdemo{i:08d}",
    }
    if cur == "jpy":
        c.update(status="failed", paid=False, captured=False)
    label = "clean"
    r = rng.random()
    if r < 0.06:
        label, fn = rng.choice(RAW_DEFECTS)
        fn(c)
    elif r < 0.09:
        c.pop("customer")
        label = "guest_checkout"
    return c, label


def check(compiled, obj) -> tuple[bool, list[dict]]:
    r = validate(compiled, obj if isinstance(obj, dict) else {})
    viol = [
        {
            "field": v.field,
            "kind": getattr(v.kind, "value", str(v.kind)),
            "message": v.message,
        }
        for v in r.violations
    ]
    return r.passed, viol


def generate_and_validate(n: int, seed: int):
    compiled = compiled_contract()
    rng = random.Random(seed)
    t0 = int(time.time())
    labels = Counter()
    kinds = Counter()
    passed, failed = [], []
    t_ns = []
    for i in range(n):
        rec, label = make_charge(i, rng, t0)
        labels[label] += 1
        t0n = time.perf_counter_ns()
        ok, viol = check(compiled, rec)
        t_ns.append(time.perf_counter_ns() - t0n)
        if ok:
            passed.append(rec)
        else:
            failed.append({"record": rec, "violations": viol, "label": label})
            for v in viol:
                kinds[f"{v['field']}:{v['kind']}"] += 1
    t_ns.sort()
    p50 = t_ns[len(t_ns) // 2] if t_ns else 0
    return passed, failed, labels, kinds, p50


def _ignore_already(fn, *args, **kwargs):
    try:
        return fn(*args, **kwargs)
    except ClientError as e:
        code = e.response.get("Error", {}).get("Code", "")
        if code in (
            "BucketAlreadyOwnedByYou",
            "BucketAlreadyExists",
            "EntityAlreadyExists",
            "AlreadyExistsException",
            "ResourceNumberLimitExceededException",
        ):
            return None
        raise


def ensure_bucket(c):
    s3 = c["s3"]
    kwargs = {"Bucket": BUCKET}
    # us-east-1 must not send a LocationConstraint.
    if REGION != "us-east-1":
        kwargs["CreateBucketConfiguration"] = {"LocationConstraint": REGION}
    try:
        s3.create_bucket(**kwargs)
        log(f"created bucket {BUCKET}")
    except ClientError as e:
        code = e.response.get("Error", {}).get("Code", "")
        if code in ("BucketAlreadyOwnedByYou", "BucketAlreadyExists"):
            log(f"bucket exists {BUCKET}")
        else:
            raise
    s3.put_public_access_block(
        Bucket=BUCKET,
        PublicAccessBlockConfiguration={
            "BlockPublicAcls": True,
            "IgnorePublicAcls": True,
            "BlockPublicPolicy": True,
            "RestrictPublicBuckets": True,
        },
    )
    s3.put_bucket_ownership_controls(
        Bucket=BUCKET,
        OwnershipControls={"Rules": [{"ObjectOwnership": "BucketOwnerEnforced"}]},
    )
    s3.put_bucket_encryption(
        Bucket=BUCKET,
        ServerSideEncryptionConfiguration={
            "Rules": [{"ApplyServerSideEncryptionByDefault": {"SSEAlgorithm": "AES256"}}]
        },
    )
    s3.put_bucket_versioning(Bucket=BUCKET, VersioningConfiguration={"Status": "Enabled"})
    s3.put_bucket_tagging(Bucket=BUCKET, Tagging={"TagSet": TAGS})
    policy = {
        "Version": "2012-10-17",
        "Statement": [
            {
                "Sid": "HttpsOnly",
                "Effect": "Deny",
                "Principal": "*",
                "Action": "s3:*",
                "Resource": [f"arn:aws:s3:::{BUCKET}", f"arn:aws:s3:::{BUCKET}/*"],
                "Condition": {"Bool": {"aws:SecureTransport": "false"}},
            }
        ],
    }
    s3.put_bucket_policy(Bucket=BUCKET, Policy=json.dumps(policy))


def empty_bucket(c):
    s3 = c["s3"]
    try:
        s3.head_bucket(Bucket=BUCKET)
    except ClientError as e:
        code = e.response.get("Error", {}).get("Code", "")
        if code in ("404", "NoSuchBucket", "NotFound"):
            log(f"bucket already gone {BUCKET}")
            return
        raise
    paginator = s3.get_paginator("list_object_versions")
    deleted = 0
    for page in paginator.paginate(Bucket=BUCKET):
        objs = []
        for v in page.get("Versions", []) + page.get("DeleteMarkers", []):
            objs.append({"Key": v["Key"], "VersionId": v["VersionId"]})
        for i in range(0, len(objs), 1000):
            chunk = objs[i : i + 1000]
            s3.delete_objects(Bucket=BUCKET, Delete={"Objects": chunk, "Quiet": True})
            deleted += len(chunk)
    log(f"emptied {deleted} object versions from {BUCKET}")


def delete_bucket(c):
    empty_bucket(c)
    try:
        c["s3"].delete_bucket(Bucket=BUCKET)
        log(f"deleted bucket {BUCKET}")
    except ClientError as e:
        code = e.response.get("Error", {}).get("Code", "")
        if code in ("404", "NoSuchBucket", "NotFound"):
            log(f"bucket already gone {BUCKET}")
        else:
            raise


def ensure_workgroup(c):
    athena = c["athena"]
    config = {
        "ResultConfiguration": {"OutputLocation": f"s3://{BUCKET}/athena-results/"},
        "EnforceWorkGroupConfiguration": True,
        "PublishCloudWatchMetricsEnabled": False,
        "BytesScannedCutoffPerQuery": 1_073_741_824,  # 1 GiB cap
        "EngineVersion": {"SelectedEngineVersion": "Athena engine version 3"},
    }
    try:
        athena.create_work_group(
            Name=WORKGROUP,
            Description="P8 NightMoose lakehouse (sandbox, do not use for prod).",
            Configuration=config,
            Tags=TAGS,
        )
        log(f"created workgroup {WORKGROUP}")
    except ClientError as e:
        code = e.response.get("Error", {}).get("Code", "")
        if code in ("InvalidRequestException", "WorkGroupAlreadyExists"):
            log(f"workgroup exists {WORKGROUP}")
        else:
            # Athena uses InvalidRequestException for "already exists"
            if "already exists" in str(e).lower():
                log(f"workgroup exists {WORKGROUP}")
            else:
                raise


def delete_workgroup(c):
    try:
        c["athena"].delete_work_group(WorkGroup=WORKGROUP, RecursiveDeleteOption=True)
        log(f"deleted workgroup {WORKGROUP}")
    except ClientError as e:
        if "is not found" in str(e).lower() or e.response.get("Error", {}).get("Code") in (
            "InvalidRequestException",
            "ResourceNotFoundException",
        ):
            log(f"workgroup already gone {WORKGROUP}")
        else:
            raise


def run_query(c, sql: str, timeout=180, use_db=True):
    athena = c["athena"]
    kwargs = {"QueryString": sql, "WorkGroup": WORKGROUP}
    # CREATE/DROP DATABASE must not set a QueryExecutionContext database
    # that does not exist yet (or is being dropped).
    if use_db:
        kwargs["QueryExecutionContext"] = {"Database": DB}
    q = athena.start_query_execution(**kwargs)
    qid = q["QueryExecutionId"]
    deadline = time.time() + timeout
    while time.time() < deadline:
        d = athena.get_query_execution(QueryExecutionId=qid)["QueryExecution"]
        state = d["Status"]["State"]
        if state in ("SUCCEEDED", "FAILED", "CANCELLED"):
            if state != "SUCCEEDED":
                reason = d["Status"].get("StateChangeReason", state)
                raise RuntimeError(f"Athena query {state}: {reason}\n{sql[:500]}")
            stats = d.get("Statistics", {})
            return qid, stats
        time.sleep(0.6)
    raise TimeoutError(f"Athena query still running: {qid}")


def query_rows(c, sql: str):
    qid, stats = run_query(c, sql)
    paginator = c["athena"].get_paginator("get_query_results")
    rows = []
    headers = None
    for page in paginator.paginate(QueryExecutionId=qid):
        for i, row in enumerate(page["ResultSet"]["Rows"]):
            data = [col.get("VarCharValue") for col in row["Data"]]
            if headers is None:
                headers = data
                continue
            rows.append(dict(zip(headers, data)))
    return rows, stats


def col_ddl():
    return ",\n  ".join(f"{n} {t}" for n, t in CHARGE_COLUMNS)


def put_jsonl(c, key: str, records: list[dict]):
    # JSONL, one object per line. None fields omitted so Athena sees NULLs.
    body = "".join(json.dumps({k: v for k, v in r.items() if v is not None}) + "\n" for r in records)
    c["s3"].put_object(
        Bucket=BUCKET,
        Key=key,
        Body=body.encode("utf-8"),
        ServerSideEncryption="AES256",
        ContentType="application/x-ndjson",
    )
    return len(body)


def setup_catalog(c):
    run_query(
        c,
        f"CREATE DATABASE IF NOT EXISTS {DB} LOCATION 's3://{BUCKET}/warehouse/{DB}/'",
        timeout=60,
        use_db=False,
    )
    # Staging tables over the JSONL we upload. Recreate each run.
    for name in ("staging_passed", "staging_failed"):
        run_query(c, f"DROP TABLE IF EXISTS {DB}.{name}", timeout=60)
        run_query(
            c,
            f"""
CREATE EXTERNAL TABLE {DB}.{name} (
  {col_ddl()}
)
ROW FORMAT SERDE 'org.openx.data.jsonserde.JsonSerDe'
WITH SERDEPROPERTIES ('ignore.malformed.json'='true')
LOCATION 's3://{BUCKET}/staging/{name}/'
""",
            timeout=60,
        )
    for name, loc in (
        ("charges_raw", f"s3://{BUCKET}/warehouse/{DB}/charges_raw/"),
        ("charges_quarantine", f"s3://{BUCKET}/warehouse/{DB}/charges_quarantine/"),
    ):
        run_query(c, f"DROP TABLE IF EXISTS {DB}.{name}", timeout=60)
        run_query(
            c,
            f"""
CREATE TABLE {DB}.{name} (
  {col_ddl()}
)
LOCATION '{loc}'
TBLPROPERTIES (
  'table_type'='ICEBERG',
  'format'='PARQUET',
  'write_compression'='SNAPPY'
)
""",
            timeout=120,
        )
    log("catalog ready (Glue database + Iceberg tables + JSON staging)")


def drop_catalog(c):
    # Best-effort. down() also empties the bucket.
    try:
        run_query(c, f"DROP DATABASE IF EXISTS {DB} CASCADE", timeout=120, use_db=False)
        log(f"dropped database {DB}")
    except Exception as e:
        log(f"drop database skipped: {type(e).__name__}")
        try:
            c["glue"].delete_database(Name=DB)
            log(f"deleted glue database {DB}")
        except ClientError:
            log(f"glue database already gone {DB}")


def load_iceberg(c, passed, failed):
    put_jsonl(c, "staging/staging_passed/passed.jsonl", passed)
    # Quarantine table has the same charge columns; store the raw record only.
    failed_recs = [item["record"] for item in failed]
    put_jsonl(c, "staging/staging_failed/failed.jsonl", failed_recs)
    cols = ", ".join(n for n, _ in CHARGE_COLUMNS)
    q1, s1 = run_query(
        c,
        f"INSERT INTO {DB}.charges_raw ({cols}) SELECT {cols} FROM {DB}.staging_passed",
        timeout=180,
    )
    q2, s2 = run_query(
        c,
        f"INSERT INTO {DB}.charges_quarantine ({cols}) SELECT {cols} FROM {DB}.staging_failed",
        timeout=180,
    )
    return {"insert_passed": s1, "insert_failed": s2, "qid_passed": q1, "qid_failed": q2}


def dq_report(c, expected_passed: int, expected_failed: int):
    queries = {
        "passed_count": f"SELECT COUNT(*) AS n FROM {DB}.charges_raw",
        "quarantine_count": f"SELECT COUNT(*) AS n FROM {DB}.charges_quarantine",
        "passed_negative_amount": f"SELECT COUNT(*) AS n FROM {DB}.charges_raw WHERE amount < 0",
        "passed_bad_currency": f"SELECT COUNT(*) AS n FROM {DB}.charges_raw WHERE currency IS NULL OR NOT regexp_like(currency, '^[a-z]{{3}}$')",
        "passed_bad_status": f"SELECT COUNT(*) AS n FROM {DB}.charges_raw WHERE status NOT IN ('succeeded','pending','failed')",
        "passed_missing_created": f"SELECT COUNT(*) AS n FROM {DB}.charges_raw WHERE created IS NULL",
        "passed_bad_id": f"SELECT COUNT(*) AS n FROM {DB}.charges_raw WHERE NOT regexp_like(id, '^(ch|py)_[A-Za-z0-9]+$')",
        "currency_drift": f"SELECT currency, COUNT(*) AS n FROM {DB}.charges_raw GROUP BY currency ORDER BY n DESC",
        "status_drift": f"SELECT status, COUNT(*) AS n FROM {DB}.charges_raw GROUP BY status ORDER BY n DESC",
        "amount_stats": f"SELECT MIN(amount) AS min_amount, MAX(amount) AS max_amount, AVG(amount) AS avg_amount FROM {DB}.charges_raw",
        "null_customer_passed": f"SELECT COUNT(*) AS n FROM {DB}.charges_raw WHERE customer IS NULL",
        "quarantine_status": f"SELECT status, COUNT(*) AS n FROM {DB}.charges_quarantine GROUP BY status ORDER BY n DESC",
    }
    out = {}
    scanned = 0
    for name, sql in queries.items():
        rows, stats = query_rows(c, sql)
        out[name] = rows
        scanned += int(stats.get("DataScannedInBytes") or 0)
    out["_bytes_scanned"] = scanned
    out["_expected_passed"] = expected_passed
    out["_expected_failed"] = expected_failed
    # Contract-drift findings: values in the passed table that should have been held.
    leaks = {
        "negative_amount": int(out["passed_negative_amount"][0]["n"]) if out["passed_negative_amount"] else 0,
        "bad_currency": int(out["passed_bad_currency"][0]["n"]) if out["passed_bad_currency"] else 0,
        "bad_status": int(out["passed_bad_status"][0]["n"]) if out["passed_bad_status"] else 0,
        "missing_created": int(out["passed_missing_created"][0]["n"]) if out["passed_missing_created"] else 0,
        "bad_id": int(out["passed_bad_id"][0]["n"]) if out["passed_bad_id"] else 0,
    }
    out["_leaks_in_passed"] = leaks
    out["_counts_match"] = (
        int(out["passed_count"][0]["n"]) == expected_passed
        and int(out["quarantine_count"][0]["n"]) == expected_failed
    )
    return out


def summarize_stats(stats: dict) -> dict:
    return {
        "data_scanned_bytes": int(stats.get("DataScannedInBytes") or 0),
        "engine_execution_ms": int(stats.get("EngineExecutionTimeInMillis") or 0),
        "total_execution_ms": int(stats.get("TotalExecutionTimeInMillis") or 0),
    }


def cmd_run(args):
    c = clients()
    ident = identity(c)
    log(f"caller {ident['Arn']} account {ident['Account']} region {REGION}")
    if ident["Account"] != ACCOUNT:
        raise SystemExit(f"unexpected account {ident['Account']}")
    t0 = time.time()
    passed, failed, labels, kinds, p50 = generate_and_validate(args.records, args.seed)
    log(
        f"validated {args.records} records: passed={len(passed)} failed={len(failed)} "
        f"check_p50_ns={p50}"
    )
    ensure_bucket(c)
    ensure_workgroup(c)
    setup_catalog(c)
    load_stats = load_iceberg(c, passed, failed)
    report = dq_report(c, expected_passed=len(passed), expected_failed=len(failed))
    elapsed = time.time() - t0
    results = {
        "project": "P8",
        "ts": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "region": REGION,
        "account": ACCOUNT,
        "caller_arn": ident["Arn"],
        "bucket": BUCKET,
        "database": DB,
        "workgroup": WORKGROUP,
        "contract": CONTRACT_NAME,
        "records": args.records,
        "seed": args.seed,
        "passed": len(passed),
        "failed": len(failed),
        "reject_rate": (len(failed) / args.records) if args.records else 0,
        "produced_labels": dict(labels),
        "violation_kinds": dict(kinds),
        "check_ns_p50": int(p50),
        "elapsed_s": round(elapsed, 2),
        "insert": {
            "passed": summarize_stats(load_stats["insert_passed"]),
            "failed": summarize_stats(load_stats["insert_failed"]),
        },
        "athena_bytes_scanned_dq": report.pop("_bytes_scanned"),
        "counts_match": report.pop("_counts_match"),
        "leaks_in_passed_iceberg": report.pop("_leaks_in_passed"),
        "dq": report,
        "notes": [
            "Write-time validation used the in-process contractgate package, not the staging or prod API.",
            "Python contractgate 0.4.x does not enforce format:email, so bad_email rows may pass ingest.",
            "Guest checkouts (no customer) are allowed by stripe_charges_raw and land in charges_raw.",
            "Glue Data Quality (R12 nmb-glue) was not used; no nmb-glue role exists, and Athena SQL covered the checks.",
        ],
    }
    out_dir = Path(args.output)
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / "run.json"
    path.write_text(json.dumps(results, indent=2) + "\n")
    md = render_markdown(results)
    (out_dir / "report.md").write_text(md)
    log(f"wrote {path} and {out_dir / 'report.md'}")
    print(json.dumps({k: results[k] for k in ("passed", "failed", "counts_match", "leaks_in_passed_iceberg", "athena_bytes_scanned_dq", "elapsed_s")}, indent=2))
    if not results["counts_match"] or any(results["leaks_in_passed_iceberg"].values()):
        sys.exit(2)


def render_markdown(r: dict) -> str:
    leaks = r["leaks_in_passed_iceberg"]
    leak_lines = "\n".join(f"| {k} | {v} |" for k, v in leaks.items())
    labels = "\n".join(f"| {k} | {v} |" for k, v in sorted(r["produced_labels"].items(), key=lambda x: -x[1]))
    kinds = "\n".join(f"| {k} | {v} |" for k, v in sorted(r["violation_kinds"].items(), key=lambda x: -x[1]))
    return f"""# P8 lakehouse contract run

- Time: {r['ts']} (UTC)
- Region: {r['region']}
- Bucket: `{r['bucket']}`
- Contract: `{r['contract']}` (in-process contractgate, not the staging or prod API)
- Records: {r['records']} (seed {r['seed']})
- Passed / quarantined: **{r['passed']} / {r['failed']}** (reject rate {r['reject_rate']:.4f})
- Iceberg counts match write-time split: {r['counts_match']}
- Athena DQ bytes scanned: {r['athena_bytes_scanned_dq']}
- Wall clock: {r['elapsed_s']} s
- Check p50: {r['check_ns_p50']} ns

## Produced labels

| label | n |
|---|---|
{labels}

## Write-time violation kinds

| kind | n |
|---|---|
{kinds}

## Athena checks on the passed Iceberg table (should all be 0)

| check | rows |
|---|---|
{leak_lines}

## Notes

{chr(10).join('- ' + n for n in r['notes'])}
"""


def cmd_down(args):
    c = clients()
    ident = identity(c)
    log(f"teardown as {ident['Arn']}")
    # Drop catalog first so we don't leave Glue tables if the bucket delete works.
    try:
        # Workgroup + bucket must exist for DROP DATABASE via Athena.
        drop_catalog(c)
    except Exception as e:
        log(f"catalog teardown warning: {type(e).__name__}: {e}")
    try:
        delete_workgroup(c)
    except Exception as e:
        log(f"workgroup teardown warning: {type(e).__name__}: {e}")
    try:
        delete_bucket(c)
    except Exception as e:
        log(f"bucket teardown warning: {type(e).__name__}: {e}")
    # Final glue fallback
    try:
        c["glue"].delete_database(Name=DB)
    except ClientError:
        pass
    log("teardown finished")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run", help="create, load, query, write results (does not tear down)")
    r.add_argument("--records", type=int, default=10000)
    r.add_argument("--seed", type=int, default=42)
    r.add_argument("--output", default=str(DEFAULT_RESULTS))
    r.set_defaults(func=cmd_run)
    d = sub.add_parser("down", help="delete the workgroup, Glue database, and bucket")
    d.set_defaults(func=cmd_down)
    args = p.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
