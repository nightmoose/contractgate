# Slack Bot Setup Guide

This doc walks through creating the ContractGate Slack app from scratch,
configuring the correct scopes, wiring it to the deployed Vercel URL, and
adding the bot to your workspace.

---

## Prerequisites

- A deployed Vercel instance of the dashboard (the `/api/slack/events` route
  must be publicly reachable)
- A Supabase project with migration `033_slack_leads.sql` applied
- The env vars described in `.env.example` set on Vercel (see Step 5)

---

## Step 1 — Create the Slack App

1. Go to **https://api.slack.com/apps** and click **Create New App**.
2. Choose **From scratch**.
3. Name it **ContractGate** (or whatever you prefer).
4. Select your target workspace and click **Create App**.

---

## Step 2 — Configure OAuth Scopes

In the sidebar, go to **OAuth & Permissions → Scopes → Bot Token Scopes**
and add the following:

| Scope | Purpose |
|-------|---------|
| `chat:write` | Post messages to channels and DMs |
| `app_mentions:read` | Receive events when the bot is @-mentioned |
| `im:read` | Receive DMs sent to the bot |
| `im:write` | Open DM conversations |
| `im:history` | Read DM message history (used for event delivery) |
| `channels:history` | Read channel history (needed for thread context in some workspaces) |

> **Note:** you do not need `channels:read` unless you want the bot to list
> channels. The above scopes are the minimum needed to receive and reply.

---

## Step 3 — Enable Events API

1. In the sidebar, go to **Event Subscriptions**.
2. Toggle **Enable Events** to **On**.
3. In the **Request URL** field enter your Vercel URL:
   ```
   https://<your-vercel-domain>/api/slack/events
   ```
   Slack will immediately send a `url_verification` POST. The handler
   responds with the `challenge` value — you should see a green **Verified** badge.

4. Under **Subscribe to bot events**, add:
   - `app_mention` — fires when someone @-mentions the bot in a channel
   - `message.im` — fires when someone DMs the bot directly

5. Click **Save Changes**.

---

## Step 4 — Install the App to Your Workspace

1. Go to **OAuth & Permissions** and click **Install to Workspace**.
2. Review and approve the requested permissions.
3. You'll be redirected back and shown a **Bot User OAuth Token** starting
   with `xoxb-`. Copy it — this is your `SLACK_BOT_TOKEN`.

---

## Step 5 — Set Environment Variables on Vercel

In your Vercel project (**Settings → Environment Variables**), add the
following server-side (non-`NEXT_PUBLIC_`) variables:

| Variable | Where to find it |
|----------|-----------------|
| `SLACK_BOT_TOKEN` | **OAuth & Permissions** → Bot User OAuth Token (`xoxb-...`) |
| `SLACK_SIGNING_SECRET` | **Basic Information** → App Credentials → Signing Secret |
| `SLACK_ANNOUNCE_SECRET` | Generate locally: `openssl rand -hex 32` |
| `LLM_PROVIDER` | `xai` or `anthropic` (default). Q&A model backend. |
| `XAI_API_KEY` | Required when `LLM_PROVIDER=xai` — https://console.x.ai/ |
| `XAI_MODEL` | Optional; default `grok-4` |
| `ANTHROPIC_API_KEY` | Required when `LLM_PROVIDER=anthropic` — https://console.anthropic.com/ |
| `ANTHROPIC_MODEL` | Optional; default `claude-sonnet-4-6` |
| `SUPABASE_SERVICE_ROLE_KEY` | Supabase dashboard → Project Settings → API |
| `NEXT_PUBLIC_SUPABASE_URL` | Supabase dashboard → Project Settings → API |

After setting variables, **redeploy** the Vercel project so they take effect.

---

## Step 6 — Add the Bot to a Channel

The bot responds to DMs automatically once installed. To have it respond to
@-mentions in a channel:

1. Open the channel in Slack.
2. Type `/invite @ContractGate` (or the name you gave the app).
3. The bot will now receive `app_mention` events from that channel.

---

## Step 7 — Test It

**DM test:**
1. Find ContractGate in your workspace's Apps section and send it a message:
   ```
   Tell me about ContractGate
   ```
   It should reply with a brief overview.

2. To test the intake flow, send:
   ```
   I'm interested in a pilot
   ```
   The bot should start walking through the 5-question intake form.

**@-mention test:**
In any channel where the bot is invited, type:
```
@ContractGate how does quarantine replay work?
```

**Announce endpoint test:**
```bash
curl -X POST https://<your-vercel-domain>/api/slack/announce \
  -H "Authorization: Bearer <SLACK_ANNOUNCE_SECRET>" \
  -H "Content-Type: application/json" \
  -d '{"channel": "#general", "message": "Hello from ContractGate!"}'
```

---

## Scheduled Announcements

The `/api/slack/announce` endpoint is designed to be called from an external
cron (GitHub Actions, Vercel Cron, etc.):

```yaml
# .github/workflows/slack-announce.yml (example)
on:
  schedule:
    - cron: '0 14 * * 1'  # Mondays at 14:00 UTC
jobs:
  announce:
    runs-on: ubuntu-latest
    steps:
      - run: |
          curl -X POST ${{ vars.VERCEL_URL }}/api/slack/announce \
            -H "Authorization: Bearer ${{ secrets.SLACK_ANNOUNCE_SECRET }}" \
            -H "Content-Type: application/json" \
            -d '{"channel": "#data-engineering", "message": "Weekly ContractGate update: ..."}'
```

---

## Troubleshooting

**"dispatch_failed" in Slack Event Subscriptions:**
- Check Vercel function logs for errors (Vercel → Deployments → Functions tab)
- Verify all env vars are set and the project was redeployed after setting them

**Bot doesn't respond to @-mentions:**
- Confirm `app_mention` is in the subscribed bot events
- Confirm the bot is invited to the channel (`/invite @ContractGate`)
- Confirm `SLACK_SIGNING_SECRET` matches the one in Slack → Basic Information

**Intake leads not saving:**
- Check that migration `033_slack_leads.sql` has been applied to Supabase
- Verify `SUPABASE_SERVICE_ROLE_KEY` is set (not the anon key)

**"url_verification" challenge fails:**
- The Vercel deployment must be live before pasting the URL into Slack
- Check that `/api/slack/events` returns 200 with `{"challenge":"..."}` via curl:
  ```bash
  curl -X POST https://<your-domain>/api/slack/events \
    -H "Content-Type: application/json" \
    -d '{"type":"url_verification","challenge":"test123"}'
  # Expected: {"challenge":"test123"}
  # Note: this bypasses signature check — Slack sends the real signature
  ```
  _(The production handler will reject unsigned requests — the above is just
  to confirm the route is reachable and parses JSON correctly.)_
