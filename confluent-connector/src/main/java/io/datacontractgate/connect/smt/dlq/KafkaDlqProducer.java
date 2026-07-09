package io.datacontractgate.connect.smt.dlq;

import org.apache.kafka.clients.producer.KafkaProducer;
import org.apache.kafka.clients.producer.Producer;
import org.apache.kafka.clients.producer.ProducerConfig;
import org.apache.kafka.clients.producer.ProducerRecord;
import org.apache.kafka.common.serialization.ByteArraySerializer;
import org.apache.kafka.connect.connector.ConnectRecord;
import org.apache.kafka.connect.sink.SinkRecord;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.Map;

/**
 * Dedicated Kafka producer for per-violation DLQ routing (RFC-064).
 *
 * <h2>Why a dedicated producer?</h2>
 * <p>Kafka Connect's {@code ErrantRecordReporter} interface (present in
 * Connect 3.6.0) routes errors to the single DLQ topic configured on the
 * connector ({@code errors.deadletterqueue.topic.name}).  It does not expose
 * a way to override the destination topic per record.  Per-violation routing
 * therefore requires the SMT to open its own producer — a well-established
 * pattern used by Debezium, Lenses, and other production SMTs.</p>
 *
 * <p>The producer is opened lazily in {@link #send} on first use and shared
 * for all subsequent calls.  It is closed in {@link #close()}.</p>
 *
 * <h2>Config keys (all prefixed {@code contractgate.dlq.routing.producer.})</h2>
 * <ul>
 *   <li>{@code bootstrap.servers} — <strong>required</strong></li>
 *   <li>Any additional key under this prefix is passed through to
 *       {@link KafkaProducer} config as-is (e.g.
 *       {@code contractgate.dlq.routing.producer.security.protocol=SSL}).</li>
 * </ul>
 *
 * <p>Serializers are fixed to {@link ByteArraySerializer} for both key and
 * value — the original record bytes are written unchanged to the DLQ topic.</p>
 */
public class KafkaDlqProducer implements AutoCloseable {

    private static final Logger log = LoggerFactory.getLogger(KafkaDlqProducer.class);

    private static final String PRODUCER_CONFIG_PREFIX = "contractgate.dlq.routing.producer.";

    private final Map<String, Object> producerProps;
    private volatile Producer<byte[], byte[]> producer;

    /**
     * Creates a new producer wrapper.
     *
     * @param allConnectorProps the full connector props map (including all
     *                          {@code contractgate.dlq.routing.producer.*} keys)
     * @param bootstrapServers  the required {@code bootstrap.servers} value
     */
    public KafkaDlqProducer(Map<String, ?> allConnectorProps, String bootstrapServers) {
        Map<String, Object> props = new HashMap<>();
        props.put(ProducerConfig.BOOTSTRAP_SERVERS_CONFIG, bootstrapServers);
        props.put(ProducerConfig.KEY_SERIALIZER_CLASS_CONFIG, ByteArraySerializer.class.getName());
        props.put(ProducerConfig.VALUE_SERIALIZER_CLASS_CONFIG, ByteArraySerializer.class.getName());
        // Idempotent delivery — ensures exactly-once writes to DLQ topics.
        props.put(ProducerConfig.ENABLE_IDEMPOTENCE_CONFIG, "true");
        props.put(ProducerConfig.ACKS_CONFIG, "all");
        props.put(ProducerConfig.CLIENT_ID_CONFIG, "contractgate-dlq-router");

        // Pass through any additional producer.* keys from connector config.
        for (Map.Entry<String, ?> e : allConnectorProps.entrySet()) {
            if (e.getKey().startsWith(PRODUCER_CONFIG_PREFIX)) {
                String subKey = e.getKey().substring(PRODUCER_CONFIG_PREFIX.length());
                if (!subKey.equals("bootstrap.servers")) { // already set above
                    props.put(subKey, e.getValue());
                }
            }
        }
        this.producerProps = props;
    }

    /**
     * Package-private constructor for testing — accepts a pre-built producer.
     */
    KafkaDlqProducer(Producer<byte[], byte[]> mockProducer) {
        this.producerProps = new HashMap<>();
        this.producer = mockProducer;
    }

    /**
     * Sends the original record value to the given DLQ topic.
     *
     * <p>The record key and value are written as raw bytes.  Connect headers
     * from the original record are not forwarded (the original record's
     * DLQ headers from Connect's own error-handling still apply to the main
     * DLQ path; this producer is only for routing-overridden topics).</p>
     *
     * @param targetTopic the resolved DLQ topic name
     * @param record      the original Connect record that failed validation
     */
    public <R extends ConnectRecord<R>> void send(String targetTopic, R record) {
        ensureProducer();

        byte[] key   = toBytes(record.key());
        byte[] value = toBytes(record.value());

        ProducerRecord<byte[], byte[]> pr = new ProducerRecord<>(targetTopic, key, value);

        producer.send(pr, (metadata, ex) -> {
            if (ex != null) {
                log.warn("Failed to write record to DLQ topic '{}': {}", targetTopic, ex.getMessage());
            } else {
                log.debug("Wrote record to DLQ topic '{}' partition={} offset={}",
                    targetTopic, metadata.partition(), metadata.offset());
            }
        });
    }

    @Override
    public void close() {
        Producer<byte[], byte[]> p = producer;
        if (p != null) {
            try {
                p.flush();
                p.close();
                log.info("KafkaDlqProducer closed");
            } catch (Exception e) {
                log.warn("Error closing KafkaDlqProducer: {}", e.getMessage());
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    private void ensureProducer() {
        if (producer == null) {
            synchronized (this) {
                if (producer == null) {
                    producer = new KafkaProducer<>(producerProps);
                    log.info("KafkaDlqProducer opened (bootstrap.servers={})",
                        producerProps.get(ProducerConfig.BOOTSTRAP_SERVERS_CONFIG));
                }
            }
        }
    }

    /** Converts a Connect record field to bytes for the producer. */
    private static byte[] toBytes(Object value) {
        if (value == null)         return null;
        if (value instanceof byte[]) return (byte[]) value;
        if (value instanceof String) return ((String) value).getBytes(StandardCharsets.UTF_8);
        return value.toString().getBytes(StandardCharsets.UTF_8);
    }
}
