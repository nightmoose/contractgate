.PHONY: demo demo-down demo-reset demo-logs stack-up stack-up-demo stack-down

# ── Demo mode (RFC-023) ───────────────────────────────────────────────────────
# Zero-auth local experience. No Supabase project, no API keys, no sign-up.
# First build: ~2-3 min.  Subsequent: instant (layer cache).
#
#   make demo          — boot full zero-auth stack
#   make demo-down     — stop + wipe volumes
#   make demo-reset    — wipe volumes + restart fresh
#   make demo-logs     — follow all service logs

demo:
	docker compose --profile demo up --build

demo-down:
	docker compose --profile demo down -v

demo-reset: demo-down demo

demo-logs:
	docker compose --profile demo logs -f

# ── Legacy aliases ────────────────────────────────────────────────────────────

stack-up:
	docker compose up

stack-up-demo:
	docker compose --profile demo up --build

stack-down:
	docker compose down
