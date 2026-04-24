package io.datacontractgate.connect.smt;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.ObjectMapper;
import io.datacontractgate.connect.client.ContractGateClient;
import io.datacontractgate.connect.client.ContractGateClient.ContractGateApiException;
import io.datacontractgate.connect.client.IngestResponse;
import io.datacontractgate.connect.client.IngestResponse.IngestEventResult;
import io.datacontractgate.connect.client.ViolationDetail;
import org.apache.kafka.common.config.ConfigDef;
import org.apache.kafka.connect.connector.ConnectRecord;
import org.apache.kafka.connect.data.Schema;
import org.apache.kafka.connect.data.Struct;
import org.apache.kafka.connect.errors.DataException;
import org.apache.kafka.connect.header.Headers;
import org.apache.kafka.connect.transforms.Transformation;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.nio.charset.StandardCharsets;
import java.util.Map;

/**
 * Kafka Connect Single Message Transform (SMT) that validates every record
 * against a ContractGate semantic contract in real-time.
 *
 * <h2>Failure actions</h2>
 * <ul>
 *   <li><b>DLQ</b> (default) — throws {@link DataException} so Kafka Connect
 *       routes the record to {@code errors.deadletterqueue.topic.name}.
 *       Enable {@code errors.deadletterqueue.context.headers.enable=true} on
 *       the connector to surface violation details in DLQ record headers.</li>
 *   <li><b>TAG_AND_PASS</b> — adds violation headers and passes the record
 *       downstream unchanged. Consumers decide what to do.</li>
 * </ul>
 *
 * <h2>Connector configuration example</h2>
 * <pre>{@code
 * "transforms": "contractgate",
 * "transforms.contractgate.type": "io.datacontractgate.connect.smt.ContractGateValidator",
 * "transforms.contractgate.contractgate.api.url": "https://api.contractgate.io",
 * "transforms.contractgate.contractgate.api.key": "${file:/opt/secrets.properties:contractgate.key}",
 * "transforms.contractgate.contractgate.contract.id": "3fa85f64-5717-4562-b3fc-2c963f66afa6"
 * }</pre>
 *
 * <p>See {@link ContractGateValidatorConfig} for the full set of options.</p>
 */
public class ContractGateValidator<R extends ConnectRecord<R>> implements Transformation<R> {

    private static final Logger log = LoggerFactory.getLogger(ContractGateValidator.class);

    // ── Header name constants ─────────────────────────────────────────────────

    private static final String H_PASSED          = "contractgate.passed";
    private static final String H_CONTRACT_VERSION = "contractgate.contract.version";
    private static final String H_VIOLATIONS_COUNT = "contractgate.violations.count";
    private static final String H_VIOLATION_PREFIX  = "contractgate.violation.";

    // ── State ─────────────────────────────────────────────────────────────────

    private ContractGateValidatorConfig config;
    private ContractGateClient client;
    private final ObjectMapper mapper = new ObjectMapper();

    // ── Transformation lifecycle ──────────────────────────────────────────────

    @Override
    public ConfigDef config() {
        return ContractGateValidatorConfig.CONFIG_DEF;
    }

    @Override
    public void configure(Map<String, ?> props) {
        this.config = new ContractGateValidatorConfig(props);
        this.client = new ContractGateClient(
            config.apiUrl(),
            config.contractId(),
            config.apiKey(),
            config.contractVersion(),
            config.dryRun(),
            config.connectTimeoutMs(),
            config.requestTimeoutMs()
        );
        log.info("ContractGateValidator configured — onFailure={} addHeaders={} dryRun={}",
            config.onFailure(), config.addResultHeaders(), config.dryRun());
    }

    @Override
    public void close() {
        // HttpClient has no close method; GC handles it.
        log.debug("ContractGateValidator closed.");
    }

    // ── Core transform ────────────────────────────────────────────────────────

    @Override
    public R apply(R record) {
        // Tombstones (value == null) bypass validation — they signal deletions.
        if (record.value() == null) {
            log.debug("Skipping tombstone record on topic={} partition={} offset={}",
                record.topic(), record.kafkaPartition(), recordOffset(record));
            return record;
        }

        String json = toJson(record);

        IngestResponse response;
        try {
            response = client.validate(json, config.requestTimeoutMs());
        } catch (ContractGateApiException e) {
            // API unreachable / bad response — fail open with a warning so a
            // transient outage does not halt the connector. Operators can tighten
            // this by setting on.failure=DLQ and letting Connect's retry logic
            // handle it at the task level if desired.
            log.warn("ContractGate API unavailable for topic={} offset={}: {}. Passing record through.",
                record.topic(), recordOffset(record), e.getMessage());
            return record;
        }

        IngestEventResult result = response.singleResult();

        if (result.passed) {
            return config.addResultHeaders()
                ? addPassHeaders(record, result)
                : record;
        }

        // ── Validation failed ─────────────────────────────────────────────────
        log.debug("Record failed validation — topic={} offset={} violations={}",
            record.topic(), recordOffset(record), result.violationSummary());

        ContractGateValidatorConfig.OnFailure onFailure = config.onFailure();
        if (onFailure == ContractGateValidatorConfig.OnFailure.DLQ) {
            // DataException causes Kafka Connect to route the original record
            // to errors.deadletterqueue.topic.name (if configured).
            throw new DataException(buildDlqMessage(record, result));
        } else {
            // TAG_AND_PASS: add violation headers and continue downstream.
            R tagged = addFailHeaders(record, result);
            log.info("TAG_AND_PASS: tagged and forwarded invalid record — topic={} offset={} {}",
                record.topic(), recordOffset(record), result.violationSummary());
            return tagged;
        }
    }

    // ── Header helpers ────────────────────────────────────────────────────────

    /**
     * Stamps {@code contractgate.passed=true} and metadata headers on a
     * passing record, then returns a new record with those headers appended.
     */
    private R addPassHeaders(R record, IngestEventResult result) {
        R newRecord = record.newRecord(
            record.topic(), record.kafkaPartition(),
            record.keySchema(), record.key(),
            record.valueSchema(), record.value(),
            record.timestamp(),
            record.headers().duplicate()
        );
        Headers headers = newRecord.headers();
        headers.addString(H_PASSED, "true");
        if (result.contractVersion != null) {
            headers.addString(H_CONTRACT_VERSION, result.contractVersion);
        }
        headers.addString(H_VIOLATIONS_COUNT, "0");
        return newRecord;
    }

    /**
     * Stamps {@code contractgate.passed=false}, violation count, and up to
     * {@code max.violation.headers} individual violation headers, then returns
     * a new record with those headers appended.
     */
    private R addFailHeaders(R record, IngestEventResult result) {
        R newRecord = record.newRecord(
            record.topic(), record.kafkaPartition(),
            record.keySchema(), record.key(),
            record.valueSchema(), record.value(),
            record.timestamp(),
            record.headers().duplicate()
        );
        Headers headers = newRecord.headers();
        headers.addString(H_PASSED, "false");
        if (result.contractVersion != null) {
            headers.addString(H_CONTRACT_VERSION, result.contractVersion);
        }
        int violationCount = result.violations == null ? 0 : result.violations.size();
        headers.addString(H_VIOLATIONS_COUNT, String.valueOf(violationCount));

        int max = config.maxViolationHeaders();
        if (result.violations != null) {
            for (int i = 0; i < Math.min(result.violations.size(), max); i++) {
                ViolationDetail v = result.violations.get(i);
                String prefix = H_VIOLATION_PREFIX + i + ".";
                if (v.field   != null) headers.addString(prefix + "field",   v.field);
                if (v.kind    != null) headers.addString(prefix + "kind",    v.kind);
                if (v.message != null) headers.addString(prefix + "message", v.message);
            }
        }
        return newRecord;
    }

    // ── Serialisation helpers ─────────────────────────────────────────────────

    /**
     * Converts a Kafka record value to a JSON string suitable for the
     * ContractGate ingest API.
     *
     * <p>Handles the three common value representations used in Kafka Connect:
     * <ol>
     *   <li>{@code String} — already JSON (or plain text); sent as-is.</li>
     *   <li>{@code byte[]} — treated as a UTF-8 JSON string.</li>
     *   <li>{@link Struct} — converted to a field→value {@link Map} then
     *       serialised to JSON.</li>
     *   <li>{@code Map} — serialised directly to JSON.</li>
     *   <li>Everything else — Jackson serialises via reflection; works for
     *       common primitives and POJO types.</li>
     * </ol>
     * </p>
     *
     * @throws DataException if serialisation fails (malformed input)
     */
    private String toJson(R record) {
        Object value = record.value();
        try {
            if (value instanceof String) {
                return (String) value;
            }
            if (value instanceof byte[]) {
                return new String((byte[]) value, StandardCharsets.UTF_8);
            }
            if (value instanceof Struct) {
                // Convert Connect Struct to a plain Map before serialising
                return mapper.writeValueAsString(structToMap((Struct) value));
            }
            // Map, primitive wrappers, or unknown types
            return mapper.writeValueAsString(value);
        } catch (JsonProcessingException e) {
            throw new DataException(
                "ContractGateValidator: failed to serialise record value to JSON " +
                "on topic=" + record.topic() + " offset=" + recordOffset(record) +
                ": " + e.getMessage(), e);
        }
    }

    /**
     * Recursively converts a Kafka Connect {@link Struct} to a
     * {@link java.util.LinkedHashMap} so Jackson can serialise it.
     */
    private Map<String, Object> structToMap(Struct struct) {
        Map<String, Object> map = new java.util.LinkedHashMap<>();
        for (org.apache.kafka.connect.data.Field field : struct.schema().fields()) {
            Object val = struct.get(field);
            if (val instanceof Struct) {
                map.put(field.name(), structToMap((Struct) val));
            } else {
                map.put(field.name(), val);
            }
        }
        return map;
    }

    // ── DLQ message ───────────────────────────────────────────────────────────

    /**
     * Builds a human-readable {@link DataException} message that Kafka Connect
     * will embed in the DLQ record's error context headers when
     * {@code errors.deadletterqueue.context.headers.enable=true}.
     */
    private String buildDlqMessage(R record, IngestEventResult result) {
        return String.format(
            "ContractGate validation failed — topic=%s partition=%s offset=%s " +
            "contract=%s version=%s %s",
            record.topic(),
            record.kafkaPartition() == null ? "-" : record.kafkaPartition(),
            recordOffset(record),
            config.contractId(),
            result.contractVersion != null ? result.contractVersion : "unknown",
            result.violationSummary()
        );
    }

    /**
     * Safely returns the Kafka offset for logging.
     * {@code kafkaOffset()} only exists on {@link org.apache.kafka.connect.sink.SinkRecord};
     * for source records (where it is not meaningful) we return {@code "-"}.
     */
    private static String recordOffset(ConnectRecord<?> record) {
        if (record instanceof org.apache.kafka.connect.sink.SinkRecord) {
            return String.valueOf(((org.apache.kafka.connect.sink.SinkRecord) record).kafkaOffset());
        }
        return "-";
    }
}
