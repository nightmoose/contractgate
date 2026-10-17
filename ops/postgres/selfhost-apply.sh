#!/bin/sh
# Apply schema after GoTrue has created the real auth.users table.
# 006 then installs the NULL auth.uid() stub; 044 replaces only that stub.
set -eu

echo "selfhost-apply: waiting for auth.users.encrypted_password"
i=0
ready=0
while [ "$i" -lt 60 ]; do
    if psql -v ON_ERROR_STOP=1 -tAc \
        "SELECT 1 FROM information_schema.columns WHERE table_schema = 'auth' AND table_name = 'users' AND column_name = 'encrypted_password'" \
        | grep -q 1; then
        ready=1
        break
    fi
    i=$((i + 1))
    sleep 2
done

if [ "$ready" -ne 1 ]; then
    echo "selfhost-apply: auth schema was not created" >&2
    exit 1
fi

echo "selfhost-apply: applying migrations"
for f in $(ls /migrations/*.sql | sort); do
    echo "  → $f"
    psql -v ON_ERROR_STOP=1 -f "$f"
done

echo "selfhost-apply: applying grants"
psql -v ON_ERROR_STOP=1 -f /selfhost/grants.sql
echo "selfhost-apply: complete"
