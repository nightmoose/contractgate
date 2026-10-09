## 1.1.0

Fixes from end-to-end testing on 2026-10-09.

- The connection test and the contract dropdown now fail with a clear message when the Gateway URL does not answer like the ContractGate API. A web page used to read as "0 contracts" and an empty dropdown.
- Test step in the Zap editor now validates the record against the gateway as a dry run, and shows the real pass or violations. It used to return static sample data without calling ContractGate.
- Contract accepts a contract name as well as the dropdown's id.
- The dropdown marks contracts that have no stable version. Running against one says to deploy a version, instead of a raw 409.
- The halt message leads with the quarantine id, which Zapier's run view used to truncate away, and no longer promises a replay that free plans cannot do.
- Clearer Record errors and help text for the Code by Zapier step.

## 1.0.0

Initial release.

- New action create/validate_record. Checks one record against a contract. A failure is quarantined and the Zap stops. A pass returns the payload the next step should write.
- New trigger trigger/new_quarantine. Fires when a record is held for failing its contract.
