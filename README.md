# SS-Billing — Soroban Contract

> The on-chain billing engine for [SS-Billing](https://github.com/brite-side0/SS-Billing) — a Soroban smart contract on the **Stellar network** that enforces subscription plans, recurring payments, grace periods, and retry logic without any intermediary.

[![Rust](https://img.shields.io/badge/Rust-stable-orange)](https://rustup.rs)
[![Soroban](https://img.shields.io/badge/Soroban-Smart%20Contract-7B2FBE)](https://soroban.stellar.org)
[![Stellar](https://img.shields.io/badge/Network-Stellar-blue)](https://stellar.org)

---

## Overview

The contract is the single source of truth for all billing state. It handles:

- Merchant and plan registration
- Subscriber authorization (sign once, billed forever)
- Recurring payment execution via SEP-41 token transfers
- Grace period and retry logic on payment failure
- Pause, resume, and cancellation lifecycle

`process_payment` is **permissionless** — any actor (scheduler, keeper, or the subscriber themselves) can trigger a due billing cycle.

---

## Data Structures

```rust
#[contracttype]
pub struct SubscriptionPlan {
    pub plan_id:      u64,
    pub merchant_id:  Address,
    pub name:         Symbol,
    pub amount:       i128,       // in stroops (1 XLM = 10_000_000)
    pub token:        Address,    // any SEP-41 token (USDC, XLM, etc.)
    pub interval:     u64,        // billing interval in seconds
    pub grace_period: u64,        // retry window after missed payment
    pub retry_limit:  u32,
    pub active:       bool,
}

#[contracttype]
pub struct Subscriber {
    pub subscriber:      Address,
    pub plan_id:         u64,
    pub next_billing_at: u64,     // Unix timestamp
    pub status:          SubscriptionStatus,
    pub retries:         u32,
    pub started_at:      u64,
}

#[contracttype]
pub enum SubscriptionStatus {
    Active,
    Paused,
    GracePeriod,
    Failed,
    Cancelled,
}
```

---

## Contract Interface

```rust
// Merchant registration
fn register_merchant(env: Env, name: Symbol, treasury_wallet: Address) -> Result<(), ContractError>
fn update_treasury(env: Env, merchant_id: Address, new_treasury: Address) -> Result<(), ContractError>

// Plan management
fn create_plan(env: Env, merchant_id: Address, name: Symbol, amount: i128,
               token: Address, interval: u64, grace_period: u64, retry_limit: u32) -> Result<u64, ContractError>
fn update_plan(env: Env, merchant_id: Address, plan_id: u64, amount: i128,
               interval: u64, grace_period: u64, retry_limit: u32) -> Result<(), ContractError>
fn disable_plan(env: Env, merchant_id: Address, plan_id: u64) -> Result<(), ContractError>

// Subscription lifecycle
fn subscribe(env: Env, subscriber: Address, plan_id: u64) -> Result<(), ContractError>
fn process_payment(env: Env, subscriber: Address, plan_id: u64) -> Result<(), ContractError>
fn pause_subscription(env: Env, subscriber: Address, plan_id: u64) -> Result<(), ContractError>
fn resume_subscription(env: Env, subscriber: Address, plan_id: u64) -> Result<(), ContractError>
fn cancel_subscription(env: Env, subscriber: Address, plan_id: u64) -> Result<(), ContractError>

// Queries
fn get_merchant(env: Env, merchant_id: Address) -> Option<Merchant>
fn get_plan(env: Env, plan_id: u64) -> Option<SubscriptionPlan>
fn get_subscriber(env: Env, subscriber: Address, plan_id: u64) -> Option<Subscriber>
fn get_merchant_plans(env: Env, merchant_id: Address) -> Vec<u64>
fn get_subscriber_plans(env: Env, subscriber: Address) -> Vec<u64>
```

---

## Payment Flow

```rust
pub fn process_payment(env: Env, subscriber: Address, plan_id: u64) -> Result<(), ContractError> {
    let mut sub = load_subscriber(&env, &subscriber, plan_id)
        .ok_or(ContractError::SubscriptionNotFound)?;

    let now = env.ledger().timestamp();
    if now < sub.next_billing_at {
        return Err(ContractError::BillingNotDue);
    }

    match Self::_transfer_payment(&env, &subscriber, &treasury, &plan.token, plan.amount) {
        Ok(_) => {
            sub.next_billing_at = now + plan.interval;
            sub.retries = 0;
            sub.status = SubscriptionStatus::Active;
            events::payment_success(&env, &subscriber, plan.amount, now);
        }
        Err(_) => {
            sub.retries += 1;
            if sub.retries >= plan.retry_limit {
                sub.status = SubscriptionStatus::Failed;
                events::payment_failed(&env, &subscriber, plan_id, sub.retries);
                return Err(ContractError::RetryLimitExceeded);
            }
            sub.status = SubscriptionStatus::GracePeriod;
        }
    }
    Ok(())
}
```

---

## On-Chain Events

The contract emits structured events consumable by off-chain indexers:

| Event | Trigger |
|-------|---------|
| `merchant_registered` | New merchant registered |
| `plan_created` | New billing plan created |
| `subscribed` | Subscriber joined a plan |
| `payment_success` | Billing cycle completed |
| `payment_failed` | Transfer failed, retry scheduled |
| `retry_attempted` | Retry attempt recorded |
| `subscription_paused` | Subscriber paused billing |
| `subscription_resumed` | Subscriber resumed billing |
| `subscription_cancelled` | Subscription permanently cancelled |

---

## Build & Deploy

```bash
# Install Rust wasm target
rustup target add wasm32-unknown-unknown

# Build
cargo build --target wasm32-unknown-unknown --release

# Run tests
cargo test

# Deploy to Stellar testnet
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/subscription_engine.wasm \
  --network testnet \
  --source <YOUR_SECRET_KEY>
```

---

## Stellar Network

| Network | RPC URL |
|---------|---------|
| Testnet | `https://soroban-testnet.stellar.org` |
| Mainnet | `https://soroban-mainnet.stellar.org` |

- Token standard: **SEP-41** (compatible with USDC, XLM, and any Stellar asset)
- Amounts are in **stroops**: `1 XLM = 10,000,000 stroops`
- Explorer: [stellar.expert](https://stellar.expert)

---

## Part of SS-Billing

| Repo | Description |
|------|-------------|
| [SS-Billing](https://github.com/brite-side0/SS-Billing) | Monorepo |
| [SS-Billing-Frontend](https://github.com/brite-side0/SS-Billing-frontend) | Next.js dashboard |
| [SS-Billing-Backend](https://github.com/brite-side0/SS-Billing-backend) | NestJS API |

---

MIT © [brite-side0](https://github.com/brite-side0)
