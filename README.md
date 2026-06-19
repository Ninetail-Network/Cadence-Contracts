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

Callers must use the appropriate variant — raw string keys are no longer accepted.

### Metrics

The `MetricsRegistry` exposes the following cache-related counters:

| Metric | Description |
|---|---|
| `cache_hits_total` | Entry found and returned |
| `cache_misses_total` | Entry not found |
| `cache_expired_total` | Entry found but TTL had elapsed (counted as miss) |
| `cache_serialization_failures_total` | Deserialization error on a cached value |

### Operational Tuning

| Variable | Default | Description |
|---|---|---|
| `CACHE_VERIFICATION_TTL` | `3600` | Seconds before a cached verification result expires |
| `REDIS_URL` | `redis://127.0.0.1:6379` | Redis connection string (production) |

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
