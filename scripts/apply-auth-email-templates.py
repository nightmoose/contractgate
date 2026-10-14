#!/usr/bin/env python3
"""Push supabase/templates/*.html to the hosted Auth email templates.

Do not run this until /auth/confirm is deployed. The templates link at the
app, and a reset email sent before that route exists will 404.

  SUPABASE_ACCESS_TOKEN=sbp_… python3 scripts/apply-auth-email-templates.py
  SUPABASE_ACCESS_TOKEN=sbp_… python3 scripts/apply-auth-email-templates.py --apply

Without --apply, prints the fields it would patch and does not call the API.
Only the five template bodies are sent. Subjects, SMTP, and secrets are left
alone.
"""

from __future__ import annotations

import json
import os
import sys
import urllib.request
from pathlib import Path

PROJECT_REF = "nmhoehpveqkkpfegkzpn"
ROOT = Path(__file__).resolve().parents[1]
TEMPLATES = {
    "mailer_templates_recovery_content": ROOT / "supabase/templates/recovery.html",
    "mailer_templates_confirmation_content": ROOT / "supabase/templates/confirmation.html",
    "mailer_templates_magic_link_content": ROOT / "supabase/templates/magic_link.html",
    "mailer_templates_email_change_content": ROOT / "supabase/templates/email_change.html",
    "mailer_templates_invite_content": ROOT / "supabase/templates/invite.html",
}


def main() -> int:
    apply = "--apply" in sys.argv
    token = os.environ.get("SUPABASE_ACCESS_TOKEN", "").strip()
    body = {key: path.read_text() for key, path in TEMPLATES.items()}
    for key, html in body.items():
        if "{{ .ConfirmationURL }}" in html or "{{ .TokenHash }}" not in html:
            print(f"refusing {key}: template must use TokenHash and not ConfirmationURL", file=sys.stderr)
            return 1
        print(f"{key}: {len(html)} bytes from {TEMPLATES[key].relative_to(ROOT)}")
    if not apply:
        print("dry run. pass --apply to patch the hosted project.")
        return 0
    if not token:
        print("SUPABASE_ACCESS_TOKEN is required for --apply", file=sys.stderr)
        return 1
    req = urllib.request.Request(
        f"https://api.supabase.com/v1/projects/{PROJECT_REF}/config/auth",
        data=json.dumps(body).encode(),
        method="PATCH",
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "User-Agent": "contractgate-apply-auth-email-templates",
        },
    )
    with urllib.request.urlopen(req) as res:
        updated = json.load(res)
    for key in body:
        got = updated.get(key, "")
        if "{{ .TokenHash }}" not in got:
            print(f"hosted {key} did not stick", file=sys.stderr)
            return 1
        print(f"updated {key}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
