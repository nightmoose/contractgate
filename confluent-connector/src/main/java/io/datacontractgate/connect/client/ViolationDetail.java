package io.datacontractgate.connect.client;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;

/**
 * A single rule violation returned by the ContractGate ingest API.
 * Matches the {@code Violation} struct in the Rust backend.
 */
@JsonIgnoreProperties(ignoreUnknown = true)
public class ViolationDetail {

    /** Dot-separated path to the offending field, e.g. {@code "customer.address.country"}. */
    public String field;

    /** Human-readable explanation, e.g. {@code "required field missing"}. */
    public String message;

    /**
     * Machine-readable violation kind. One of:
     * missing_required_field, type_mismatch, pattern_mismatch,
     * enum_violation, range_violation, length_violation,
     * metric_range_violation, undeclared_field.
     */
    public String kind;

    @Override
    public String toString() {
        return field + " [" + kind + "]: " + message;
    }
}
