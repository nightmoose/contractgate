# DataContractGate plugin

DataContractGate rejects events that are schema-valid but wrong, such as a misspelled `event_type` or a negative `amount`, before they reach your Kafka topic. This plugin lets Claude or Cursor infer a semantic data contract from real sample events in your repo, dry-run it, deploy it, and inspect quarantined events.

## What's included

- **Skill** (`skills/contractgate`): the procedure the agent follows to wire contracts into a repo.
- **MCP server** (`@nightmoose/contractgate-mcp-server`, run with `npx -y`, unpinned so it tracks the current npm release; 0.4.1 as of 2026-11-14): six tools — `infer_contract`, `validate_events`, `deploy_contract`, `get_quarantine`, `list_contracts`, and `egress_validate`.

## Setup

Validate one sample with no key at https://app.datacontractgate.com/playground (`POST https://api.datacontractgate.com/public/playground/validate`). The MCP tools need a key.

1. Create an API key at https://app.datacontractgate.com/account.
2. Install the plugin.
   - **Claude Code:** `/plugin marketplace add nightmoose/contractgate`, then `/plugin install contractgate@contractgate`. Claude Code asks for the API key and stores it in your system's secure credential store.
   - **Cursor:** install from the Cursor Marketplace and enter the key when prompted.

## What it sends and where

The MCP server calls the DataContractGate gateway at `https://api.datacontractgate.com` (override with `CONTRACTGATE_BASE_URL`) with your API key. Create the key and read the docs at `https://app.datacontractgate.com`. The server sends only what the tools are given: sample events for inference, events for validation, and contract YAML for deployment. Validation defaults to a dry run, which records nothing. Nothing else leaves your machine, and the plugin has no hooks or background processes.

Full playbook: https://app.datacontractgate.com/llm-integration.md. Tool reference: https://app.datacontractgate.com/mcp-reference.md.

## Privacy, terms, and support

- Privacy policy: https://app.datacontractgate.com/privacy
- Terms of service: https://app.datacontractgate.com/terms
- Support: https://github.com/nightmoose/contractgate/issues

## License

MIT
