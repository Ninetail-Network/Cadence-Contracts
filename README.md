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

<<<<<<< HEAD
### 🛡️ Rate Limiting

**Why Rate Limiting?** Without rate limits, attackers can spam the contract with thousands of
registrations/revocations, polluting state and causing denial-of-service conditions. ProofStell
enforces **per-issuer** and **per-address** rate limits to prevent abuse while maintaining
fair access for legitimate high-volume users.

#### Rate Limit Architecture

The contract uses a **token bucket algorithm**:

- Each issuer and address gets a **bucket** of tokens that refills at a fixed rate per second
- Registrations and revocations **consume 1 token** each
- When a bucket is empty, operations are rejected with `RateLimitExceeded`
- Tokens refill over time, allowing burst operations

#### On-Chain Configuration

These constants are compiled into the contract and require redeployment to change:

```rust
const ISSUER_RATE_LIMIT_PER_SECOND: u64 = 100;     // Per-issuer refill rate
const ISSUER_RATE_LIMIT_BURST: u64 = 100;          // Per-issuer max tokens
const ADDRESS_RATE_LIMIT_PER_SECOND: u64 = 50;     // Per-address refill rate
const ADDRESS_RATE_LIMIT_BURST: u64 = 50;          // Per-address max tokens
const OPERATION_COST: u64 = 1;                      // Tokens per operation
```

**Default Behavior:**
- Each issuer can register/revoke **100 documents instantly** (burst), then at **100/sec** sustained
- Each address (owner) can have documents registered **50 times instantly**, then at **50/sec** sustained
- Separate limits are **independent** — one issuer reaching the limit doesn't affect others

#### Service-Side Configuration

For HTTP service deployments, configure rate limits via environment variables:

```bash
# Global rate limit (service-level, not contract)
export RATE_LIMIT_PER_SECOND=10
export RATE_LIMIT_BURST=10

# Per-issuer rate limit
export RATE_LIMIT_PER_ISSUER_PER_SECOND=100
export RATE_LIMIT_PER_ISSUER_BURST=100

# Per-address rate limit
export RATE_LIMIT_PER_ADDRESS_PER_SECOND=50
export RATE_LIMIT_PER_ADDRESS_BURST=50
```

#### Tuning for Production

**High-Volume Issuers:**

If your users register >100 documents/sec on average, increase constants before deployment:

```rust
const ISSUER_RATE_LIMIT_PER_SECOND: u64 = 1000;    // 1000 ops/sec sustained
const ISSUER_RATE_LIMIT_BURST: u64 = 1000;         // 1000 instant operations
```

Redeploy the contract:
```bash
cargo build --target wasm32-unknown-unknown --release
soroban contract deploy --wasm target/wasm32-unknown-unknown/release/proofstell_contract.wasm --network testnet
```

**Conservative (Testing/Small Scale):**

```rust
const ISSUER_RATE_LIMIT_PER_SECOND: u64 = 10;
const ISSUER_RATE_LIMIT_BURST: u64 = 10;
const ADDRESS_RATE_LIMIT_PER_SECOND: u64 = 5;
const ADDRESS_RATE_LIMIT_BURST: u64 = 5;
```

#### Error Handling

When rate limits are exceeded, the contract returns:

```
ContractError::RateLimitExceeded (code: 7)
```

Clients should:

1. **Retry with exponential backoff** (1s, 2s, 4s, ...)
2. **Monitor metrics** to detect systematic rate limit issues
3. **Request quota increase** if sustained demand exceeds configured limits

#### Metrics & Monitoring

Prometheus metrics track rate limit behavior:

- `rate_limit_tokens_consumed_total` — Total tokens consumed across all limits
- `rate_limit_violations_total` — Total requests rejected (global)
- `issuer_rate_limit_violations_total` — Per-issuer violations
- `address_rate_limit_violations_total` — Per-address violations
- `issuer_rate_limit_resets_total` — Bucket refills after exhaustion
- `address_rate_limit_resets_total` — Bucket refills after exhaustion

**Example Prometheus Query:**
```promql
# Rate of rate limit violations in the last 5 minutes
rate(rate_limit_violations_total[5m])

# Percentage of requests hitting rate limit
(rate(rate_limit_violations_total[1m]) / rate(requests_total[1m])) * 100
```

If violations spike, you may need to:
- Increase rate limit thresholds (redeploy contract)
- Identify and block abusive callers
- Scale infrastructure to handle higher request volume
=======
### 🔄 Upgrades & Governance

* Single-admin governance — one address (set at `initialize`) controls upgrades, migrations, and feature flags
* Contract version stored in persistent ledger — survives ledger entry expiry
* Feature flags allow toggling behaviours without a full WASM upgrade
* `ContractInitialized` and `ContractUpgraded` events let indexers detect which contract version produced any given document event

---

### 📦 Batch Operations

* `batch_register_documents` — register up to 20 documents in one transaction
* `batch_revoke_documents` — revoke up to 20 documents in one transaction

**Atomicity:** All documents succeed or none are written. If any item in the batch fails (e.g. duplicate hash, wrong issuer, already revoked), the entire call returns an error and no state is changed.

**Batch size limit:** Maximum 20 documents per call. Exceeding this returns `BatchTooLarge` (error code 7). Empty batches return `BatchEmpty` (error code 8).

**Fee implications:** A single transaction covers the entire batch regardless of size, making bulk operations significantly cheaper than individual calls. For best results, pre-validate document uniqueness and existence client-side before submitting to avoid wasted transaction fees on partial failures.
>>>>>>> e0fc42a42093127d66bf07a090c78c21587a3874

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

### Initialize After Deployment

After deploying, call `initialize` to set the admin address and record version 1 on-chain:

```bash
soroban contract invoke \
  --id <CONTRACT_ID> \
  --source <ADMIN_SECRET_KEY> \
  --network testnet \
  -- initialize \
  --admin <ADMIN_ADDRESS>
```

---

### Upgrade Procedure

1. **Build the new WASM** and upload it to the ledger:

```bash
cargo build --target wasm32-unknown-unknown --release
soroban contract install \
  --wasm target/wasm32-unknown-unknown/release/proofstell_contract.wasm \
  --network testnet
# Note the returned WASM hash
```

2. **Call `upgrade`** with the new WASM hash:

```bash
soroban contract invoke \
  --id <CONTRACT_ID> \
  --source <ADMIN_SECRET_KEY> \
  --network testnet \
  -- upgrade \
  --admin <ADMIN_ADDRESS> \
  --new_wasm_hash <WASM_HASH>
```

3. **Call `migrate`** to apply any data transformations and bump the version:

```bash
soroban contract invoke \
  --id <CONTRACT_ID> \
  --source <ADMIN_SECRET_KEY> \
  --network testnet \
  -- migrate \
  --admin <ADMIN_ADDRESS>
```

### Rollback Plan

Soroban contract upgrades are irreversible on-chain — there is no undo. To roll back:

1. Keep the previous WASM hash recorded before upgrading.
2. If the new version is broken, call `upgrade` again with the old WASM hash.
3. If the migration mutated storage in an incompatible way, a compensating migration must be written into the rolled-back WASM.

**Recommendation:** always test upgrades on testnet before applying to mainnet. See [docs/UPGRADE_GOVERNANCE.md](docs/UPGRADE_GOVERNANCE.md) for the full decision process.

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
