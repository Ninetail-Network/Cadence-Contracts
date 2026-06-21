---
name: upgrade-governance-design
description: Design spec for contract upgrade mechanism, versioning, migration path, and governance for ProofStell Soroban contract (issue #29)
metadata:
  type: project
---

# Contract Upgrade & Migration Governance Design

**Issue:** #29 — Add contract upgrade and migration path governance

## Problem

The ProofStell contract has no upgrade mechanism. Future changes to `DocumentRecord`, error types, or event formats would require redeploying to a new address, losing all historical state and forcing clients to migrate manually. There is no version signal for indexers to detect which contract version produced a given event.

## Approach

Single-admin governance: one `Address` set at initialization controls upgrades, migrations, and feature flags. This is the standard Soroban pattern — simple, auditable, and appropriate for an open-source contract at this maturity level. Multi-sig governance can be layered on top by pointing the admin at a multisig contract.

## Storage Layout

Three new `DataKey` variants added alongside the existing `Document(BytesN<32>)`:

| Key | Type | Description |
|-----|------|-------------|
| `DataKey::Admin` | `Address` | The governance admin address |
| `DataKey::Version` | `u32` | Current contract version (absent = 0 = pre-versioned) |
| `DataKey::FeatureFlag(Symbol)` | `bool` | Named feature toggles |

All stored in **persistent** storage to survive ledger expiry.

## New Error Codes

| Variant | Code | Description |
|---------|------|-------------|
| `AlreadyInitialized` | 9 | `initialize` called when version key already exists |
| `NotInitialized` | 10 | Governance call before `initialize` |
| `Unauthorized` | 11 | Caller is not the stored admin |
| `MigrationNotNeeded` | 12 | `migrate` called when already on latest version |

## New Entry Points

### `initialize(env, admin: Address) -> Result<(), ContractError>`
- Requires admin auth.
- Fails with `AlreadyInitialized` if `DataKey::Version` already exists in storage.
- Stores admin address and sets version to 1.
- Emits `ContractInitialized { admin, version: 1 }`.

### `upgrade(env, admin: Address, new_wasm_hash: BytesN<32>) -> Result<(), ContractError>`
- Requires admin auth. Validates against stored admin.
- Fails with `NotInitialized` if no version key exists.
- Records old version, calls `env.deployer().update_current_contract_wasm(new_wasm_hash)`.
- Emits `ContractUpgraded { admin, old_version, new_version: old_version }`.
- Note: version number is NOT auto-incremented on upgrade — `migrate` does that. This separates code deployment from data migration.

### `migrate(env, admin: Address) -> Result<u32, ContractError>`
- Requires admin auth. Validates against stored admin or accepts admin arg when version=0 (bootstrapping).
- Detects current version from storage (absent = 0).
- Applies the appropriate migration step:
  - `0 → 1`: Pre-versioned state — stores admin and bumps version to 1. (Handles existing deployments upgraded via Stellar account authority.)
  - `1 → current`: No-op for now; placeholder for future version transitions.
  - Already at latest: returns `MigrationNotNeeded`.
- Emits no extra event (the version bump is visible via `get_version`).
- Returns the new version number.

### `get_version(env) -> u32`
- Returns the stored version, or 0 if not initialized. Never fails.

### `get_admin(env) -> Option<Address>`
- Returns the stored admin address, or `None` if not initialized.

### `set_feature_flag(env, admin: Address, flag: Symbol, enabled: bool) -> Result<(), ContractError>`
- Admin-only. Stores `DataKey::FeatureFlag(flag) → enabled`.

### `get_feature_flag(env, flag: Symbol) -> bool`
- Returns the stored value, or `false` if not set.

## New Events

```rust
#[contractevent(topics = ["init"], data_format = "vec")]
pub struct ContractInitialized {
    #[topic]
    pub admin: Address,
    pub version: u32,
}

#[contractevent(topics = ["upgrade"], data_format = "vec")]
pub struct ContractUpgraded {
    #[topic]
    pub admin: Address,
    pub old_version: u32,
    pub new_version: u32,
}
```

Indexers use these events as version markers. Events between two `ContractUpgraded` events were produced by the same contract version — no per-event version field needed on existing events.

## Tests

| Test | Scenario |
|------|----------|
| `initialize_sets_admin_and_version` | Fresh init → admin stored, version=1 |
| `double_initialize_fails` | Second init → `AlreadyInitialized` |
| `get_version_returns_zero_before_init` | Uninitialized → `get_version` returns 0 |
| `get_admin_returns_none_before_init` | Uninitialized → `get_admin` returns None |
| `non_admin_cannot_upgrade` | Wrong address → `Unauthorized` |
| `upgrade_requires_initialization` | No init → `NotInitialized` |
| `migrate_v0_to_v1_sets_versioning` | Pre-versioned state → migrate → version=1 |
| `migrate_already_current_fails` | Initialized at v1 → migrate → `MigrationNotNeeded` |
| `admin_can_set_and_get_feature_flag` | Set flag → get flag returns true |
| `get_feature_flag_returns_false_for_unset` | Unset flag → false |
| `non_admin_cannot_set_feature_flag` | Wrong address → `Unauthorized` |
| `documents_still_work_after_migration` | Register doc → migrate → doc still verifiable |

## Documentation

### README additions
- Upgrade procedure (upload new WASM hash → call `upgrade` → call `migrate`)
- Rollback plan (Soroban upgrades are irreversible on-chain; rollback = re-upgrade to old WASM hash)
- Environment/deployment notes

### New file: `docs/UPGRADE_GOVERNANCE.md`
- Decision process for breaking vs non-breaking changes
- Who can propose, review, and authorize upgrades
- Emergency upgrade procedure

## Files Changed

| File | Change |
|------|--------|
| `src/lib.rs` | New DataKey variants, error codes, events, and 7 new entry points + 12 tests |
| `README.md` | Upgrade & Migration section |
| `docs/UPGRADE_GOVERNANCE.md` | New governance document |
