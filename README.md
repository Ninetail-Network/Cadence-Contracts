# 📜 ProofStell Smart Contract

Decentralized document verification contract built with Soroban.

---

## 🌍 Overview

This smart contract powers the **on-chain verification layer** of ProofStell.

It stores **cryptographic hashes of documents** and enables:

* Document registration
* Verification
* Revocation

---

## 🚀 Core Features

### 📄 Document Registry

* Store document hashes on-chain
* Ensure immutability

---

### 🔎 Verification

* Check if a document exists
* Confirm authenticity
* Cross-reference with Stellar Horizon for on-chain proof

#### Verification Proof Source

ProofStell uses a **dual-source verification model**:

1. **Stellar Horizon** (primary) — The service queries `GET /transactions?memo={hash}`
   against the configured Horizon instance. When a matching transaction is found
   with a confirmed memo match, the transaction ID and ledger timestamp are returned
   as authoritative proof.

2. **On-chain contract state** (secondary) — The Soroban contract's `verify_document`
   method confirms whether a document record exists and is `Active` in persistent
   storage.

Horizon verification distinguishes four result categories:

| Status | Meaning |
|---|---|
| `ConfirmedMatch` | A Stellar transaction with matching memo was found — proof is authoritative |
| `NoMatch` | Horizon was reachable but no transaction matches the hash |
| `NetworkError` | All retries exhausted due to connection or HTTP errors |
| `MalformedResponse` | Horizon returned a response that could not be parsed |

Only `ConfirmedMatch` constitutes a positive verification. All other results
are treated as non-verified (the document may still be valid on-chain, but no
Horizon proof exists).

---

### 🧾 Revocation

* Allow issuers to revoke documents
* Maintain revocation state

---

## 🧠 How It Works

1. Document is hashed (SHA256)

2. Hash is submitted to contract

3. Contract stores:

   * Issuer address
   * Owner address
   * Timestamp
   * Status

4. Verification compares hash with stored record

---

## 🗂️ Data Model


DocumentHash → DocumentRecord


## 🔐 Security

* No raw documents stored on-chain
* Duplicate prevention
* Issuer authorization
* Immutable records
* Revocation tracking

---

## 🛠️ Tech Stack

* Rust
* Soroban SDK
* Stellar Network

---

## 🚀 Development

### Requirements

* Rust
* Soroban CLI

---

### Install Soroban CLI

```bash
cargo install soroban-cli
```

---

### Build Contract

```bash
cargo build --target wasm32-unknown-unknown --release
```

---

### Deploy Contract

```bash
soroban contract deploy \
--wasm target/wasm32-unknown-unknown/release/proofstell_contract.wasm \
--network testnet
```

---

## 🧪 Testing

```bash
cargo test
```

---

## 🗄️ Cache Behavior

### TTL Enforcement

Both the in-memory and Redis backends honor TTL values:

- **Redis** — uses `SET EX` so entries are natively evicted after `ttl` seconds.
- **InMemory** — stores an `expires_at` timestamp alongside each value. A `get` that finds an expired entry returns a cache miss (same semantics as Redis).

The TTL for verification results is controlled by the `CACHE_VERIFICATION_TTL` environment variable (default: `3600` seconds).

### Typed Cache Keys

Cache keys are typed via the `CacheKey` enum to prevent namespace collisions:

| Variant | Prefix | Example |
|---|---|---|
| `CacheKey::Verification(hash)` | `verification:` | `verification:e3b0c4…` |
| `CacheKey::Config(key)` | `config:` | `config:rate_limit` |

Callers must use the appropriate variant — raw string keys are no longer accepted.### Metrics

The `MetricsRegistry` (defined in `src/metrics.rs`) is the central instrumentation hub for the ProofStell service layer. All service modules emit metrics through this registry, which exposes a Prometheus-compatible text-format endpoint at `/metrics`.

#### General Request Metrics

| Metric | Type | Description |
|---|---|---|
| `requests_total` | Counter | Total number of API requests |
| `errors_total` | Counter | Total number of errors encountered |

#### Cache Metrics

| Metric | Type | Description |
|---|---|---|
| `cache_hits_total` | Counter | Entry found and returned |
| `cache_misses_total` | Counter | Entry not found |
| `cache_expired_total` | Counter | Entry found but TTL had elapsed (counted as miss) |
| `cache_serialization_failures_total` | Counter | Deserialization error on a cached value |

#### Document Registration & Revocation Metrics

| Metric | Type | Labels | Description |
|---|---|---|---|
| `document_registration_total` | CounterVec | `status` (success/error) | Total document registrations by outcome |
| `document_revocation_total` | CounterVec | `status` (success/error) | Total document revocations by outcome |

#### Verification Metrics

| Metric | Type | Labels | Description |
|---|---|---|---|
| `verification_total` | CounterVec | `status` (success/failure) | Total verifications by outcome |
| `verification_latency_seconds` | HistogramVec | `status` | End-to-end verification latency in seconds |
| `horizon_latency_seconds` | HistogramVec | `status` (success/error) | Stellar Horizon API call latency in seconds |
| `retry_total` | Counter | — | Total number of retry attempts across all operations |

#### Rate Limiter Metrics

| Metric | Type | Description |
|---|---|---|
| `rate_limit_tokens_consumed_total` | Counter | Total rate limiter tokens consumed |
| `rate_limit_violations_total` | Counter | Total rate limit violations (requests rejected) |

#### Event Ingestion Metrics

| Metric | Type | Description |
|---|---|---|
| `event_duplicates_total` | Counter | Total duplicate events detected and discarded |
| `event_ordering_failures_total` | Counter | Total events rejected due to ordering/sequence failures |
| `event_backlog_size` | Gauge | Current number of unprocessed events in the backlog queue |

#### Config Metrics

| Metric | Type | Description |
|---|---|---|
| `config_validation_failures_total` | Counter | Total configuration validation failures |
| `config_reload_total` | Counter | Total configuration reloads attempted |

#### Recommended Alerting Thresholds

| Alert | Condition | Severity |
|---|---|---|
| High error rate | `rate(errors_total[5m] / requests_total[5m]) > 0.1` | Critical |
| Low cache hit rate | `rate(cache_hits_total[5m]) / rate(cache_hits_total[5m] + cache_misses_total[5m]) < 0.5` | Warning |
| High verification failure rate | `rate(verification_total{status="failure"}[5m]) > 0.05` | Warning |
| Rate limit violations spike | `rate(rate_limit_violations_total[5m]) > 10` | Warning |
| Event backlog growing | `event_backlog_size > 1000` | Warning |
| Config validation failures | `increase(config_validation_failures_total[5m]) > 0` | Critical |
| High Horizon latency | `histogram_quantile(0.95, rate(horizon_latency_seconds_bucket[5m])) > 5` | Warning |

#### Running with Metrics

Build the service binary (non-WASM target):

```bash
cargo build --release
```

The `/metrics` endpoint is served by the application HTTP server. To scrape metrics with Prometheus, add a scrape config:

```yaml
scrape_configs:
  - job_name: 'proofstell'
    static_configs:
      - targets: ['localhost:8080']
    metrics_path: '/metrics'
```

### Environment Reference

| Variable | Default | Validation / Description |
|---|---|---|
| `PORT` | `8080` | Must be a valid port from `1` to `65535` |
| `STELLAR_HORIZON_URL` | `https://horizon-testnet.stellar.org` | Must parse as a valid URL |
| `STELLAR_SECRET_KEY` | required | Must be a valid Stellar ed25519 secret key |
| `REDIS_URL` | `redis://127.0.0.1:6379` | Must parse as `redis://` or `rediss://` |
| `RATE_LIMIT_PER_SECOND` | `10` | Must be greater than `0` |
| `RATE_LIMIT_BURST` | same as `RATE_LIMIT_PER_SECOND` | Must be greater than `0` |
| `STELLAR_MAX_RETRIES` | `3` | Must be a valid unsigned integer |
| `LOG_LEVEL` | `info` | Log verbosity string |
| `WEBHOOK_URLS` | empty | Comma-separated list of valid URLs |
| `WEBHOOK_SECRET` | unset | Optional webhook signing secret |
| `CACHE_VERIFICATION_TTL` | `3600` | Seconds before a cached verification result expires |

Set `REDIS_URL` to a real Redis instance in production. The in-memory backend is suitable for local development and testing only.

## 🧾 Audit Trail

The audit trail bridges Soroban contract activity and off-chain service records through `src/event.rs`.

- Contract-origin events use deterministic idempotency keys in the form `contract:<tx_hash>:<ledger_sequence>:<event_index>:<aggregate_id>:<event_type>`.
- Contract-origin events derive monotonic sequence numbers from the ledger sequence and event index so replayed Horizon deliveries can be ordered consistently.
- Service-origin events still use generated record IDs, but can override sequence and idempotency keys when a persistence layer has stable ordering context.
- Contract metadata captures the transaction hash, ledger sequence, event index, and document hash so retries can be de-duplicated safely.

Audit records should be retained for as long as the operator needs replay and forensic traceability. On-chain contract events remain the canonical source of truth, while the off-chain audit store keeps the derived trail for search, retention, and replay handling.

---

## 🧪 Future Improvements

* Issuer registry system
* Multi-signature verification
* Zero-knowledge proofs
* Credential NFTs

---

## 🎯 Goal

To provide a **trustless, immutable verification layer** for documents using blockchain.

---

**ProofStell Contract — Trust anchored on-chain.**
