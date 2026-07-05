# Cadence Contracts (Soroban)

This is the on-chain layer of **Cadence**, a Stellar-native platform where
fans send creators anonymous messages, creators can respond by text or by
having an AI voice agent (EchoCall) call the fan back, and both sides build
a daily check-in streak (Pulse) that pays out rewards. Full project context
lives in the [top-level README](../README.md) — this document covers only
the smart contracts that make the platform trustworthy without a central
authority holding anyone's funds or identity.

These contracts exist so that:
- A fan's **payment to unlock priority attention is never just handed to the
  creator on faith** — it sits in escrow until the creator actually acts.
- A user's **daily streak can't be faked or edited by a backend** — it's a
  public, append-only record on Stellar that anyone can verify.
- **Reward issuance is automatic and auditable** — streak milestones mint the
  `VEIL` token directly, with no manual "here's your reward" step a backend
  could get wrong or skip.

Smart contracts that power **The Veil** (anonymous message escrow) and
**Pulse** (daily streak ritual + reward token), written in Rust for
[Soroban](https://soroban.stellar.org/), Stellar's smart contract platform.

## Contracts

```
contract/
├── Cargo.toml
├── veil-escrow/        ← anonymous message payments & unlock logic
│   └── src/lib.rs
├── pulse/              ← daily check-in streak tracker + reward payouts
│   └── src/lib.rs
└── veil-token/           ← SEP-41 fungible token (VEIL) minted as streak rewards
    └── src/lib.rs
```

## 1. `veil-token` — the VEIL reward token

A standard SEP-41 (Soroban token interface) fungible token. `pulse` is
granted mint authority so it can pay out streak rewards autonomously.

```rust
pub trait TokenTrait {
    fn initialize(e: Env, admin: Address, decimal: u32, name: String, symbol: String);
    fn mint(e: Env, to: Address, amount: i128);      // restricted to `pulse` contract
    fn balance(e: Env, id: Address) -> i128;
    fn transfer(e: Env, from: Address, to: Address, amount: i128);
}
```

## 2. `veil-escrow` — anonymous message + unlock payments

Holds a fan's optional payment in escrow when they submit a message, and
releases it to the creator only once the creator takes an action (reply,
publish, or trigger an EchoCall). Prevents pay-to-spam without ever storing
the fan's identity — the contract only knows a pseudonymous Stellar address.

```rust
#[contracttype]
pub struct Message {
    pub id: u64,
    pub creator: Address,
    pub sender: Address,      // fan's wallet — never linked to real identity off-chain
    pub amount: i128,          // escrowed unlock payment, 0 if free message
    pub token: Address,        // asset used (native XLM SAC or USDC SAC)
    pub status: MessageStatus, // Pending | Answered | Published | Refunded
    pub content_hash: BytesN<32>, // hash of off-chain encrypted message body
}

#[contracttype]
pub enum MessageStatus { Pending, Answered, Published, Refunded }

pub trait VeilEscrowTrait {
    /// Fan submits a message; if amount > 0, funds are pulled into escrow.
    fn submit_message(
        e: Env,
        sender: Address,
        creator: Address,
        amount: i128,
        token: Address,
        content_hash: BytesN<32>,
    ) -> u64;

    /// Creator marks a message answered — releases escrow to creator.
    fn answer_message(e: Env, creator: Address, message_id: u64);

    /// Creator publishes an anonymized answer publicly (emits event only).
    fn publish_message(e: Env, creator: Address, message_id: u64);

    /// Fan can reclaim funds if the creator ignores the message past `timeout_ledger`.
    fn refund_expired(e: Env, message_id: u64);

    fn get_message(e: Env, message_id: u64) -> Message;
}
```

Design notes:
- `content_hash` is the only content-related data on-chain — the actual
  message text is stored off-chain (backend DB, encrypted) so the ledger
  never contains raw personal writing.
- Escrow timeout (`timeout_ledger`) protects fans from creators who never
  respond; funds auto-refund after N ledgers.
- Cross-contract call into `veil-token` (or the native XLM Stellar Asset
  Contract) handles the actual token transfer.

## 3. `pulse` — daily streak check-in

```rust
#[contracttype]
pub struct StreakInfo {
    pub current_streak: u32,
    pub longest_streak: u32,
    pub last_checkin_day: u64,   // days since epoch
    pub total_checkins: u64,
}

pub trait PulseTrait {
    /// One check-in per address per calendar day. Increments streak if
    /// `last_checkin_day == today - 1`, resets to 1 if a day was missed
    /// beyond the grace period, no-ops if already checked in today.
    fn checkin(e: Env, who: Address);

    fn get_streak(e: Env, who: Address) -> StreakInfo;

    /// Called internally on milestone streaks (7/30/100/365 days) to mint
    /// VEIL rewards via cross-contract call to `veil-token::mint`.
    fn claim_milestone(e: Env, who: Address, milestone: u32);

    /// Admin-configurable grace period (default 1 day) before a streak resets.
    fn set_grace_period(e: Env, admin: Address, days: u32);
}
```

Streak milestone reward table (example, tune via governance/admin):

| Streak length | VEIL reward | Extra perk |
|---|---|---|
| 7 days | 10 VEIL | Badge NFT: "Week One" |
| 30 days | 75 VEIL | Priority queue in Veil inbox |
| 100 days | 400 VEIL | Free EchoCall credit |
| 365 days | 2000 VEIL | Non-transferable "Year of Pulse" badge |

## Building & testing

```bash
# from contract/
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
cargo test
```

## Deploying to testnet

```bash
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/veil_token.wasm \
  --source alice \
  --network testnet

soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/veil_escrow.wasm \
  --source alice \
  --network testnet

soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/pulse.wasm \
  --source alice \
  --network testnet
```

After deployment, wire up permissions:
1. Set `pulse` as the mint-authority admin on `veil-token`.
2. Set `veil-escrow`'s accepted tokens (native XLM SAC address + USDC SAC address).

## Security TODO before any mainnet use

- [ ] Reentrancy audit on cross-contract calls (`answer_message` → token transfer)
- [ ] Formal review of streak grace-period logic for timezone/day-boundary edge cases
- [ ] Rate limiting on `submit_message` to prevent escrow-griefing spam
- [ ] Independent audit of `veil-token` mint authority scope
