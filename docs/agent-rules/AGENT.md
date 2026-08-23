# ContractGate rules (generic agent instructions)

Portable version for Claude Code, Codex, Aider, Antigravity, or any agent
whose instruction format is plain markdown. Paste into your project's agent
instructions file (e.g. `CLAUDE.md`, `AGENTS.md`, `.aider.conf.yml`).

---

This repository ships event data behind ContractGate. Every event that leaves
a producer must satisfy a YAML contract in the `contracts/` directory.

**When editing an event publisher, ingestion path, or schema that feeds one:**

1. Read `contracts/<name>.yaml` for the affected event. If none exists, infer
   one: `contractgate infer --from-stdin --name <name> --out
   contracts/<name>.yaml` (feed real sample events on stdin).
2. Reflect every new or renamed producer field in the contract in the same
   change.
3. Run `contractgate test --contract contracts/<name>.yaml --data <fixture>`
   before committing. Fix violations by tightening the code, not by loosening
   the contract.
4. To deploy: `contractgate deploy-contract contracts/<name>.yaml --json`.
   Never edit a deployed version in place — bump `version:` in the YAML and
   redeploy.

Full playbook: <https://app.datacontractgate.com/llm-integration.md>.
