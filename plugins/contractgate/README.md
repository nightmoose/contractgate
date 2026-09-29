# ContractGate plugin

ContractGate rejects events that are schema-valid but wrong, such as a misspelled `event_type` or a negative `amount`, before they reach your Kafka topic. This plugin lets Claude or Cursor infer a semantic data contract from real sample events in your repo, dry-run it, deploy it, and inspect quarantined events.

## What's included

- **Skill** (`skills/contractgate`): the procedure the agent follows to wire contracts into a repo.
- **MCP server** (`@contractgate/mcp-server@0.1.2`, run with `npx`): five tools, `infer_contract`, `validate_events`, `deploy_contract`, `get_quarantine`, and `list_contracts`.

## Setup

1. Create an API key at https://app.datacontractgate.com/account.
2. Install the plugin.
   - **Claude Code:** `/plugin marketplace add nightmoose/contractgate`, then `/plugin install contractgate@contractgate`. Claude Code asks for the API key and stores it in your system's secure credential store.
   - **Cursor:** install from the Cursor Marketplace and enter the key when prompted.

## What it sends and where

The MCP server calls the ContractGate API at `https://app.datacontractgate.com` (or the base URL you configure) with your API key. It sends only what the tools are given: sample events for inference, events for validation, and contract YAML for deployment. Validation defaults to a dry run, which records nothing. Nothing else leaves your machine, and the plugin has no hooks or background processes.

Full playbook: https://app.datacontractgate.com/llm-integration.md. Tool reference: https://app.datacontractgate.com/mcp-reference.md.

## License

MIT
