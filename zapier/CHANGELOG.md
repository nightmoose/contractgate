## 1.0.0

Initial release.

- New action create/validate_record. Checks one record against a contract. A failure is quarantined and the Zap stops. A pass returns the payload the next step should write.
- New trigger trigger/new_quarantine. Fires when a record is held for failing its contract.
