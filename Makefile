.PHONY: demo demo-down demo-reset demo-logs stack-up stack-up-demo stack-down demo-ralph demo-ralph-down demo-ralph-smoke demo-ralph-e2e demo-ralph-run demo-ralph-run-mcp

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

# ── Ralph-native demo (RFC-092) ───────────────────────────────────────────────
# ContractGate gate in front of Driftless/Kafi (design-partner stack).
#   make demo-ralph        — start Redpanda + print next steps
#   make demo-ralph-smoke  — Stage A automated smoke (quarantine check)
#   make demo-ralph-down   — stop + wipe volumes

demo-ralph:
	docker compose -f demo/ralph/docker-compose.yml up -d
	@echo ""
	@echo "Redpanda up. Next (in demo/ralph, with venv):"
	@echo "  python bridge/gate_bridge.py          # terminal A"
	@echo "  python produce/producers.py           # terminal B"
	@echo "  python produce/inject_bad.py          # terminal C"
	@echo "Console: http://localhost:8088"
	@echo "See demo/ralph/README.md"

demo-ralph-smoke:
	bash demo/ralph/scripts/smoke.sh

demo-ralph-e2e:
	bash demo/ralph/scripts/e2e.sh

demo-ralph-run:
	bash demo/ralph/scripts/run_demo.sh

demo-ralph-run-mcp:
	bash demo/ralph/scripts/run_demo.sh --with-mcp

demo-ralph-down:
	docker compose -f demo/ralph/docker-compose.yml down -v

# ── Legacy aliases ────────────────────────────────────────────────────────────

stack-up:
	docker compose up

stack-up-demo:
	docker compose --profile demo up --build

stack-down:
	docker compose down
