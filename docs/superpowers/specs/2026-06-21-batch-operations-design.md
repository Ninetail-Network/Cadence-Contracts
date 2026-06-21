---
name: batch-operations-design
description: Design spec for batch_register_documents and batch_revoke_documents on the ProofStell Soroban contract
metadata:
  type: project
---

# Batch Operations Design

**Issue:** #30 — Implement batch operations for bulk registration and revocation

## Problem

The contract only exposes single-document `register_document` and `revoke_document` entry points. Registering N documents requires N separate transactions, multiplying fees, latency, and the risk of partial on-chain state.

## Approach

Strict fail-fast atomicity. Both batch functions validate the full input upfront, then iterate — returning an error on the first failure. Because Soroban aborts and rolls back the entire transaction on any contract error, no partial state is ever written. This is the simplest model and exactly matches "all succeed or all fail."

Batch size is capped at **20 documents** to stay within Soroban instruction and storage operation limits. Empty batches are rejected to avoid ambiguous success responses.

## Data Model Changes

### New contracttype — `DocumentInfo`

```rust
#[contracttype]
pub struct DocumentInfo {
    pub owner: Address,
    pub document_hash: BytesN<32>,
}
```

Used as the element type for `batch_register_documents`. Groups owner and hash together so the vector is a single typed parameter (Soroban does not support tuple parameters in contract function signatures).

### New error codes

| Variant        | Code | Description                              |
|----------------|------|------------------------------------------|
| `BatchTooLarge`| 7    | Input vector exceeds the 20-item limit   |
| `BatchEmpty`   | 8    | Input vector is empty                    |

These extend `ContractError` without renumbering existing codes.

## New Entry Points

### `batch_register_documents`

```rust
pub fn batch_register_documents(
    env: Env,
    issuer: Address,
    documents: Vec<DocumentInfo>,
) -> Result<Vec<DocumentRecord>, ContractError>
```

- Calls `issuer.require_auth()` once for the whole batch.
- Returns `BatchEmpty` if `documents.len() == 0`.
- Returns `BatchTooLarge` if `documents.len() > 20`.
- Iterates in order; on any error (e.g. `AlreadyRegistered`) returns immediately.
- On full success: persists all records, emits one `DocumentRegistered` event per document, returns `Vec<DocumentRecord>`.

### `batch_revoke_documents`

```rust
pub fn batch_revoke_documents(
    env: Env,
    issuer: Address,
    document_hashes: Vec<BytesN<32>>,
) -> Result<Vec<DocumentRecord>, ContractError>
```

- Calls `issuer.require_auth()` once for the whole batch.
- Returns `BatchEmpty` if `document_hashes.len() == 0`.
- Returns `BatchTooLarge` if `document_hashes.len() > 20`.
- Iterates in order; on any error (`DocumentNotFound`, `OnlyIssuerCanRevoke`, `AlreadyRevoked`) returns immediately.
- On full success: updates all records to `Revoked`, emits one `DocumentRevoked` event per document, returns `Vec<DocumentRecord>`.

## Error Handling

All existing per-document errors (`AlreadyRegistered`, `DocumentNotFound`, `OnlyIssuerCanRevoke`, `AlreadyRevoked`) propagate unchanged from within the batch loop. The caller receives the first error that occurred; no document index is embedded in the error (Soroban error codes are `u32` scalars — no payload). Clients should validate uniqueness and existence before submitting large batches to avoid wasted fees.

## Events

One event is emitted per document, identical in structure to the single-document events:
- `DocumentRegistered { issuer, owner, document_hash }` per registered document.
- `DocumentRevoked { issuer, document_hash }` per revoked document.

No batch-level event is emitted — downstream consumers already process individual document events and do not need a separate batch envelope.

## Tests

Located inline in `src/lib.rs` (matching existing test structure):

| Test | Scenario |
|------|----------|
| `batch_register_all_succeed` | 3 distinct documents register successfully, all returned Active |
| `batch_register_fails_on_duplicate` | 1 of 3 is already registered — entire batch fails, others not stored |
| `batch_register_rejects_empty` | empty vec → `BatchEmpty` |
| `batch_register_rejects_oversized` | 21 items → `BatchTooLarge` |
| `batch_revoke_all_succeed` | 3 registered documents revoked, all returned Revoked |
| `batch_revoke_fails_on_missing` | 1 of 3 not registered — entire batch fails |
| `batch_revoke_fails_on_wrong_issuer` | different issuer → `OnlyIssuerCanRevoke`, nothing revoked |
| `batch_revoke_fails_on_already_revoked` | 1 already revoked → `AlreadyRevoked`, nothing else revoked |
| `batch_revoke_rejects_empty` | empty vec → `BatchEmpty` |
| `batch_revoke_rejects_oversized` | 21 items → `BatchTooLarge` |

## README Changes

Add a **Batch Operations** section documenting:
- Function signatures and parameter types.
- The 20-document limit and the rationale (Soroban instruction budget).
- Atomicity guarantee and fee implications (one transaction regardless of batch size).
- Recommendation to pre-validate uniqueness/existence client-side.

## Files to Change

| File | Change |
|------|--------|
| `src/lib.rs` | Add `DocumentInfo`, `BatchTooLarge`/`BatchEmpty` errors, `batch_register_documents`, `batch_revoke_documents`, and all tests |
| `README.md` | Add Batch Operations section |
