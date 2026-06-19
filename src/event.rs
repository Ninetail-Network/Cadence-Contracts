extern crate alloc;

use alloc::{
    format,
    string::{String, ToString},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Source of an audit event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventSource {
    Contract,
    Service,
}

/// Contract execution details used to derive stable audit metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractEventContext {
    pub transaction_hash: String,
    pub ledger_sequence: u64,
    pub event_index: u32,
    pub document_hash: String,
}

impl ContractEventContext {
    fn validate(&self) -> crate::error::Result<()> {
        if self.transaction_hash.trim().is_empty() {
            return Err(crate::error::AuditError::InvalidContractEventContext(
                "transaction hash cannot be empty".to_string(),
            ));
        }

        if self.document_hash.trim().is_empty() {
            return Err(crate::error::AuditError::InvalidContractEventContext(
                "document hash cannot be empty".to_string(),
            ));
        }

        if self.ledger_sequence == 0 {
            return Err(crate::error::AuditError::InvalidContractEventContext(
                "ledger sequence must be greater than 0".to_string(),
            ));
        }

        Ok(())
    }

    pub fn idempotency_key(&self, aggregate_id: &str, event_type: &str) -> String {
        format!(
            "contract:{}:{}:{}:{}:{}",
            self.transaction_hash, self.ledger_sequence, self.event_index, aggregate_id, event_type
        )
    }

    pub fn sequence(&self) -> u64 {
        self.ledger_sequence
            .saturating_mul(1_000)
            .saturating_add(u64::from(self.event_index))
    }
}

/// Core event type for audit trail.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    /// Unique event record ID.
    pub id: String,
    /// Document or aggregate ID being modified.
    pub aggregate_id: String,
    /// Type of event, such as `DocumentRegistered` or `DocumentRevoked`.
    pub event_type: String,
    /// Event payload as JSON.
    pub data: serde_json::Value,
    /// Timestamp when event occurred.
    pub timestamp: DateTime<Utc>,
    /// Sequential event number for ordering within an aggregate.
    pub sequence: u64,
    /// User or system that triggered the event.
    pub actor: String,
    /// Deterministic key used to de-duplicate replays.
    pub idempotency_key: String,
    /// Audit source that produced this event.
    pub source: EventSource,
    /// Additional metadata.
    pub metadata: Option<serde_json::Value>,
}

impl Event {
    /// Create a new service-side event.
    pub fn new(
        aggregate_id: String,
        event_type: String,
        data: serde_json::Value,
        actor: String,
    ) -> Self {
        let id = Uuid::new_v4().to_string();
        Self {
            id: id.clone(),
            aggregate_id,
            event_type,
            data,
            timestamp: Utc::now(),
            sequence: 1,
            actor,
            idempotency_key: id,
            source: EventSource::Service,
            metadata: None,
        }
    }

    /// Override the sequence number when the caller has stable ordering context.
    pub fn with_sequence(mut self, sequence: u64) -> Self {
        self.sequence = sequence;
        self
    }

    /// Override the idempotency key when the caller has a deterministic replay key.
    pub fn with_idempotency_key(mut self, idempotency_key: impl Into<String>) -> Self {
        self.idempotency_key = idempotency_key.into();
        self
    }

    /// Override the audit source.
    pub fn with_source(mut self, source: EventSource) -> Self {
        self.source = source;
        self
    }

    /// Add metadata to the event.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Create an audit event from a Soroban contract event.
    pub fn from_contract_event(
        aggregate_id: String,
        event_type: String,
        data: serde_json::Value,
        actor: String,
        context: ContractEventContext,
    ) -> crate::error::Result<Self> {
        context.validate()?;

        let id = Uuid::new_v4().to_string();
        let idempotency_key = context.idempotency_key(&aggregate_id, &event_type);
        let metadata = serde_json::json!({
            "transaction_hash": context.transaction_hash,
            "ledger_sequence": context.ledger_sequence,
            "event_index": context.event_index,
            "document_hash": context.document_hash,
            "source": "contract",
        });

        Ok(Self {
            id,
            aggregate_id,
            event_type,
            data,
            timestamp: Utc::now(),
            sequence: context.sequence(),
            actor,
            idempotency_key,
            source: EventSource::Contract,
            metadata: Some(metadata),
        })
    }

    /// Serialize event to JSON string.
    pub fn to_json(&self) -> crate::error::Result<String> {
        serde_json::to_string(self)
            .map_err(|e| crate::error::AuditError::SerializationError(e.to_string()))
    }

    /// Deserialize event from JSON string.
    pub fn from_json(json: &str) -> crate::error::Result<Self> {
        serde_json::from_str(json)
            .map_err(|e| crate::error::AuditError::SerializationError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_event_uses_non_zero_sequence() {
        let event = Event::new(
            "doc-1".to_string(),
            "Created".to_string(),
            serde_json::json!({"title": "Test"}),
            "user-1".to_string(),
        );

        assert_eq!(event.sequence, 1);
        assert_eq!(event.source, EventSource::Service);
        assert!(!event.idempotency_key.is_empty());
    }

    #[test]
    fn contract_event_derives_stable_metadata() {
        let context = ContractEventContext {
            transaction_hash: "tx123".to_string(),
            ledger_sequence: 42,
            event_index: 3,
            document_hash: "doc-hash".to_string(),
        };

        let event = Event::from_contract_event(
            "doc-1".to_string(),
            "DocumentRegistered".to_string(),
            serde_json::json!({"status": "active"}),
            "issuer-1".to_string(),
            context,
        )
        .expect("contract event should build");

        assert_eq!(event.source, EventSource::Contract);
        assert_eq!(event.sequence, 42_003);
        assert_eq!(
            event.idempotency_key,
            "contract:tx123:42:3:doc-1:DocumentRegistered"
        );
        let metadata = event.metadata.expect("contract event metadata");
        assert_eq!(metadata["transaction_hash"], "tx123");
        assert_eq!(metadata["ledger_sequence"], 42);
        assert_eq!(metadata["event_index"], 3);
        assert_eq!(metadata["document_hash"], "doc-hash");
    }

    #[test]
    fn event_serialization_roundtrip_keeps_audit_fields() {
        let context = ContractEventContext {
            transaction_hash: "tx123".to_string(),
            ledger_sequence: 42,
            event_index: 3,
            document_hash: "doc-hash".to_string(),
        };

        let event = Event::from_contract_event(
            "doc-1".to_string(),
            "DocumentRegistered".to_string(),
            serde_json::json!({"title": "Test"}),
            "issuer-1".to_string(),
            context,
        )
        .expect("contract event should build");

        let json = event.to_json().unwrap();
        let deserialized = Event::from_json(&json).unwrap();

        assert_eq!(event.id, deserialized.id);
        assert_eq!(event.aggregate_id, deserialized.aggregate_id);
        assert_eq!(event.idempotency_key, deserialized.idempotency_key);
        assert_eq!(event.sequence, deserialized.sequence);
        assert_eq!(event.source, deserialized.source);
    }

    #[test]
    fn invalid_contract_context_is_rejected() {
        let err = Event::from_contract_event(
            "doc-1".to_string(),
            "DocumentRegistered".to_string(),
            serde_json::json!({"title": "Test"}),
            "issuer-1".to_string(),
            ContractEventContext {
                transaction_hash: String::new(),
                ledger_sequence: 0,
                event_index: 0,
                document_hash: String::new(),
            },
        )
        .expect_err("context should fail validation");

        assert!(err.to_string().contains("invalid contract event context"));
    }
}
