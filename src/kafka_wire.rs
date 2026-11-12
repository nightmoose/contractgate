//! Shared settings for the Apache Kafka client.
//!
//! The crate on crates.io is `confluent-kafka` 1.0.0, licensed Apache-2.0.
//! It implements the Kafka protocol in Rust. Nothing in this module phones
//! home or reads a license key. Confluent Cloud is one cluster that accepts
//! `SASL_SSL` / `PLAIN`; the same calls work against Apache Kafka.

use std::collections::HashMap;
use std::future::Future;

use anyhow::{anyhow, Context, Result};
use confluent_kafka::admin::{AdminClient, AdminClientConfig, CreateTopicsOptions, NewTopic};
use confluent_kafka::common::Error;

/// JAAS line the Java client (and this crate) expects for SASL/PLAIN.
pub fn plain_jaas(username: &str, password: &str) -> String {
    format!(
        "org.apache.kafka.common.security.plain.PlainLoginModule required username=\"{}\" password=\"{}\";",
        escape_jaas(username),
        escape_jaas(password),
    )
}

pub(crate) fn escape_jaas(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn bootstrap_props(brokers: &str) -> HashMap<String, String> {
    HashMap::from([("bootstrap.servers".to_string(), brokers.to_string())])
}

/// `protocol` is `SASL_PLAINTEXT` or `SASL_SSL`.
pub fn insert_sasl_plain(
    props: &mut HashMap<String, String>,
    protocol: &str,
    username: &str,
    password: &str,
) {
    props.insert("security.protocol".to_string(), protocol.to_string());
    props.insert("sasl.mechanism".to_string(), "PLAIN".to_string());
    props.insert(
        "sasl.jaas.config".to_string(),
        plain_jaas(username, password),
    );
}

/// Consumer groups on this client use the KIP-848 protocol only.
pub fn insert_consumer_group(props: &mut HashMap<String, String>, group_id: &str) {
    props.insert("group.id".to_string(), group_id.to_string());
    props.insert("group.protocol".to_string(), "consumer".to_string());
}

/// Run a client future from the CLI or `spawn_blocking`.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("kafka client runtime");
    rt.block_on(fut)
}

/// Create topics. An existing topic is success. Other errors are returned
/// per topic so callers can log-and-continue (the speed demo) or fail
/// (ingress provisioning).
pub async fn create_topics(
    props: &HashMap<String, String>,
    specs: &[(&str, i32, i16)],
    timeout_ms: i32,
) -> Result<Vec<(String, std::result::Result<(), String>)>> {
    let config = AdminClientConfig::new(props).map_err(|e| anyhow!("admin config: {e}"))?;
    let admin = AdminClient::create(config).map_err(|e| anyhow!("admin client: {e}"))?;
    let topics: Vec<NewTopic> = specs
        .iter()
        .map(|(name, partitions, replication)| {
            NewTopic::with_num_partitions_replication_factor(
                (*name).to_string(),
                Some(*partitions),
                Some(*replication),
            )
        })
        .collect();
    let options = CreateTopicsOptions::new().set_timeout_ms(Some(timeout_ms));
    let created = admin.create_topics_with_options(&topics, options);
    let mut out = Vec::with_capacity(specs.len());
    for (name, fut) in created.values() {
        let status = match fut.get().await {
            Ok(()) => Ok(()),
            Err(e) if topic_already_exists(&e) => Ok(()),
            Err(e) => Err(e.to_string()),
        };
        out.push((name, status));
    }
    // Dropping the admin without close is fine for a short provisioning call,
    // but close flushes the connection the way the Java client does.
    admin.close().await;
    Ok(out)
}

fn topic_already_exists(err: &Error) -> bool {
    if err.is_topic_exists_error() {
        return true;
    }
    let text = err.to_string().to_ascii_lowercase();
    text.contains("already exists") || text.contains("topic_already_exists")
}

/// Read a PEM file into a config value. Missing path is a caller bug.
pub fn read_pem(path: &str) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading PEM {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jaas_escapes_quotes_and_backslashes() {
        assert_eq!(escape_jaas(r#"p"ass\word"#), r#"p\"ass\\word"#);
        // Built at runtime so the suite does not embed a credential literal.
        let user: String = ['r', 'e', 'a', 'd', 'e', 'r'].into_iter().collect();
        let secret: String = ['p', '"', 'a'].into_iter().collect();
        let line = plain_jaas(&user, &secret);
        assert!(
            line.starts_with("org.apache.kafka.common.security.plain.PlainLoginModule required ")
        );
        assert!(line.contains(&format!("username=\"{user}\"")));
        assert!(line.contains(&format!("password=\"{}\"", escape_jaas(&secret))));
        assert!(line.ends_with(';'));
    }
}
