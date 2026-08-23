# Agent rule presets

Drop-in rule files for coding agents that operate on repos using ContractGate.
Each file tells the agent to consult the contracts in `contracts/` and run
`contractgate test` before committing changes that affect event publishers,
ingestion paths, or schemas.

Copy the file that matches your tool into the destination path below. All
presets carry the same core rule — pick the one your team uses.

| Agent | Preset | Destination |
|---|---|---|
| Cursor | `cursor.mdc` | `.cursor/rules/contractgate.mdc` |
| Windsurf | `windsurf.md` | `.windsurf/rules/contractgate.md` |
| GitHub Copilot | `copilot-instructions.md` | `.github/copilot-instructions.md` |
| Claude Code, Codex, others | `AGENT.md` | Paste into your project's agent instructions file |

Every preset points the agent at the canonical playbook:
<https://app.datacontractgate.com/llm-integration.md>. Nothing here duplicates
that content — the presets are short by design so they can be dropped into
a repo without becoming a maintenance burden.
