# Cadence Contract Upgrade Governance

This document defines the decision process for upgrading the ProofStell Soroban contract, classifying change types, and authorizing governance actions.

---

## Change Classification

### Non-Breaking Changes (Safe to deploy without client migration)

- Adding new contract entry points
- Adding new event types
- Adding new error codes
- Adding new feature flags
- Internal refactors that preserve existing function signatures and storage layout

### Breaking Changes (Require coordinated client migration)

- Renaming or removing existing entry points
- Changing the type or field order of `DocumentRecord`, `DocumentInfo`, or any `#[contracttype]`
- Changing existing event topic strings or data fields
- Renumbering existing error codes
- Changing the storage key layout (`DataKey` variants)

Any breaking change must follow the full upgrade process below.

---

## Governance Roles

| Role | Responsibility |
|------|----------------|
| **Admin** | The address stored at `DataKey::Admin`. Authorized to call `upgrade`, `migrate`, and `set_feature_flag`. |
| **Maintainers** | Open-source contributors with merge rights on this repository. Propose and review changes. |
| **Community** | GitHub Issues and Discussions — the venue for raising concerns before any upgrade. |

---

## Upgrade Decision Process

### For Non-Breaking Changes

1. Open a PR with the change.
2. At least one maintainer reviews and approves.
3. Merge to `main`. Deploy at the next release window.

### For Breaking Changes

1. **Proposal** — Open a GitHub Issue labelled `breaking-change` describing:
   - What is changing and why
   - Which clients or indexers are affected
   - The proposed migration window (minimum 2 weeks for mainnet)
2. **Discussion window** — At least 7 days for community feedback.
3. **PR review** — Two maintainer approvals required.
4. **Testnet dry-run** — Run the full upgrade procedure on testnet and verify:
   - All existing tests pass against the new WASM
   - `migrate` completes without error
   - Document verification still works for pre-migration records
5. **Mainnet upgrade** — Admin calls `upgrade` then `migrate` on mainnet.
6. **Client migration notice** — Post a GitHub Discussion and update client documentation.

---

## Emergency Upgrade Procedure

For critical security fixes that cannot wait for the standard process:

1. Admin calls `upgrade` with the patched WASM on testnet immediately.
2. Verify no data corruption with a smoke test.
3. Admin calls `upgrade` on mainnet.
4. Open a GitHub Issue retroactively documenting the emergency change within 24 hours.
5. A follow-up PR with tests and documentation is required within 72 hours.

---

## Incompatible Storage Layout Changes

If a new version requires a `DocumentRecord` field to be added or removed:

1. The old storage key format must be read and transformed in `migrate`.
2. A new `DataKey` variant should be introduced for the new format; do not reuse old keys.
3. After migration completes, the old keys may be cleaned up in a subsequent release.
4. Tests must cover reading old-format records and verifying they are correctly transformed.

---

## Transferring Admin

To transfer governance to a new address, the current admin must call `initialize` is not re-callable — the admin address can only change by deploying a new contract or by the current admin upgrading to a contract version that supports admin transfer. Add an `transfer_admin(env, admin, new_admin)` entry point in a future version if this is needed.

---

## Audit Requirements

Before any mainnet upgrade that introduces new entry points or modifies existing logic, a security review is strongly recommended. See the [security review checklist](.github/SECURITY.md) if available.
