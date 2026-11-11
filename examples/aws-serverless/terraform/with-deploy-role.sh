#!/usr/bin/env bash
# Run a command as nmb-terraform-deploy without overwriting the caller's
# AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY. Credentials live in a temp
# file mode 600 and are deleted on exit. Never prints secrets.
set -euo pipefail

ROLE_ARN="${NMB_TF_ROLE_ARN:-arn:aws:iam::065614567889:role/nmb-terraform-deploy}"
SESSION="${NMB_TF_SESSION:-terraform-p5}"

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 2
fi

tmp=$(mktemp -d)
chmod 700 "$tmp"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT

# Assume using the caller's env (nightmoose-bots on the box). JSON stays in $tmp.
aws sts assume-role \
  --role-arn "$ROLE_ARN" \
  --role-session-name "$SESSION" \
  --duration-seconds 3600 \
  --output json > "$tmp/assume.json"
chmod 600 "$tmp/assume.json"

python3 - "$tmp" << 'PY'
import json, os, sys
d = json.load(open(os.path.join(sys.argv[1], "assume.json")))
c = d["Credentials"]
path = os.path.join(sys.argv[1], "credentials")
with open(path, "w") as f:
    f.write("[nmb-terraform-session]\n")
    f.write(f"aws_access_key_id = {c['AccessKeyId']}\n")
    f.write(f"aws_secret_access_key = {c['SecretAccessKey']}\n")
    f.write(f"aws_session_token = {c['SessionToken']}\n")
os.chmod(path, 0o600)
print(d["AssumedRoleUser"]["Arn"], "expires", c["Expiration"], file=sys.stderr)
PY
rm -f "$tmp/assume.json"

export AWS_SHARED_CREDENTIALS_FILE="$tmp/credentials"
export AWS_PROFILE=nmb-terraform-session
export AWS_SDK_LOAD_CONFIG=1
unset AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN
# AWS_DEFAULT_REGION stays set from the caller.

"$@"
