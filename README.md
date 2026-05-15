# SS-Billing

> A production-grade, decentralized subscription billing platform built on the **Stellar Soroban** smart contract platform — a trustless, on-chain alternative to Stripe Billing.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Stellar Network](https://img.shields.io/badge/Network-Stellar%20Soroban-7B2FBE)](https://stellar.org)
[![Built with Rust](https://img.shields.io/badge/Contract-Rust%20%2B%20Soroban-orange)](https://soroban.stellar.org)
[![NestJS](https://img.shields.io/badge/API-NestJS-red)](https://nestjs.com)
[![Next.js](https://img.shields.io/badge/Frontend-Next.js%2014-black)](https://nextjs.org)

---

## Overview

SS-Billing enables merchants to create subscription plans and collect recurring payments from subscribers — entirely enforced by a Soroban smart contract on the Stellar network. No intermediaries. No custodial risk. Every billing cycle is transparent, auditable, and immutable on-chain.

**Key capabilities:**

- **On-chain billing engine** — Soroban contract handles plan creation, subscriptions, payment execution, grace periods, and retry logic
- **One-time subscriber authorization** — subscribers sign once; the contract executes all future cycles permissionlessly
- **Automated scheduler** — NestJS backend polls due subscriptions and triggers `process_payment` on-chain
- **Webhook delivery** — HMAC-signed event notifications for payment success/failure
- **Analytics API** — revenue tracking and subscription metrics per merchant
- **Freighter wallet integration** — browser-native Stellar wallet support in the frontend

---

## Architecture

```
SS-Billing/
├── contracts/
│   └── subscription-engine/     # Soroban smart contract (Rust)
│       ├── src/
│       │   ├── lib.rs            # Contract entry points
│       │   ├── types.rs          # Data structures
│       │   ├── storage.rs        # Ledger storage helpers
│       │   ├── errors.rs         # ContractError enum
│       │   └── events.rs         # On-chain event emitters
│       └── tests/
├── backend/                      # NestJS API
│   └── src/
│       ├── modules/
│       │   ├── billing/          # Scheduler + blockchain indexer
│       │   ├── merchant/         # Merchant CRUD
│       │   ├── plan/             # Plan management
│       │   ├── subscription/     # Subscription lifecycle
│       │   ├── analytics/        # Revenue & stats
│       │   ├── webhook/          # HMAC-signed event delivery
│       │   └── auth/             # JWT + Stellar signature auth
│       ├── db/                   # TypeORM entities + data source
│       └── config/               # App, Stellar, Redis config
├── frontend/                     # Next.js 14 app
│   └── src/
│       ├── app/                  # App Router pages
│       ├── components/           # UI components + charts
│       ├── store/                # Zustand wallet state
│       └── lib/                  # API client
├── packages/
│   ├── types/                    # Shared TypeScript types
│   ├── sdk/                      # Contract interaction SDK
│   ├── ui/                       # Shared React components
│   └── config/                   # Network constants
└── infra/
    ├── docker/                   # Dockerfiles + docker-compose
    └── monitoring/               # Prometheus + Grafana
```

---

## How It Works

```
Subscriber                  Soroban Contract              Merchant Treasury
    │                              │                              │
    │──── subscribe(plan_id) ─────▶│                              │
    │                              │──── transfer(amount) ───────▶│  (first payment)
    │                              │                              │
    │         [time passes — billing interval elapses]            │
    │                              │                              │
Scheduler ── process_payment() ──▶│                              │
                                   │──── transfer(amount) ───────▶│  (recurring)
                                   │                              │
                                   │──── emit PaymentSuccess ─────▶ Webhook → Merchant
```

The contract enforces all business logic: grace periods, retry limits, pause/resume state, and cancellation. The backend scheduler is a convenience layer — any actor can call `process_payment` once billing is due.

---

## Smart Contract

### Core Data Types

```rust
// types.rs
#[contracttype]
pub struct SubscriptionPlan {
    pub plan_id:      u64,
    pub merchant_id:  Address,
    pub name:         Symbol,
    pub amount:       i128,       // in stroops (1 XLM = 10_000_000)
    pub token:        Address,    // any SEP-41 token
    pub interval:     u64,        // billing interval in seconds
    pub grace_period: u64,        // seconds after due date before retry
    pub retry_limit:  u32,
    pub active:       bool,
}

#[contracttype]
pub struct Subscriber {
    pub subscriber:      Address,
    pub plan_id:         u64,
    pub next_billing_at: u64,
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

### Contract Interface

```rust
// Register as a merchant
fn register_merchant(env: Env, name: Symbol, treasury_wallet: Address) -> Result<(), ContractError>

// Create a billing plan
fn create_plan(
    env: Env,
    merchant_id: Address,
    name: Symbol,
    amount: i128,
    token: Address,
    interval: u64,
    grace_period: u64,
    retry_limit: u32,
) -> Result<u64, ContractError>

// Subscribe and pay first cycle
fn subscribe(env: Env, subscriber: Address, plan_id: u64) -> Result<(), ContractError>

// Execute a due billing cycle (permissionless)
fn process_payment(env: Env, subscriber: Address, plan_id: u64) -> Result<(), ContractError>

// Lifecycle management
fn pause_subscription(env: Env, subscriber: Address, plan_id: u64) -> Result<(), ContractError>
fn resume_subscription(env: Env, subscriber: Address, plan_id: u64) -> Result<(), ContractError>
fn cancel_subscription(env: Env, subscriber: Address, plan_id: u64) -> Result<(), ContractError>
```

### Payment Execution Flow

```rust
// From lib.rs — process_payment core logic
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

## API Reference

Base URL: `http://localhost:3001/api/v1` · Swagger UI: `http://localhost:3001/docs`

### Merchants

```http
POST   /merchants                    # Register merchant
GET    /merchants/:id                # Get merchant profile
PATCH  /merchants/:id                # Update merchant
```

### Plans

```http
POST   /plans                        # Create subscription plan
GET    /plans?merchantId=<id>        # List plans for merchant
PATCH  /plans/:id                    # Update plan
DELETE /plans/:id                    # Disable plan
```

### Subscriptions

```http
POST   /subscriptions                # Create subscription
GET    /subscriptions/:id            # Get subscription details
PATCH  /subscriptions/:id/pause      # Pause billing
PATCH  /subscriptions/:id/resume     # Resume billing
DELETE /subscriptions/:id            # Cancel subscription
GET    /subscriptions/:id/payments   # Payment history
```

### Analytics

```http
GET    /analytics/merchants/:id/stats    # MRR, churn, active count
GET    /analytics/merchants/:id/revenue  # Revenue time series
```

### Example: Create a Plan

```bash
curl -X POST http://localhost:3001/api/v1/plans \
  -H "Authorization: Bearer <JWT>" \
  -H "Content-Type: application/json" \
  -d '{
    "merchantId": "GXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
    "name": "Pro Monthly",
    "amount": "100000000",
    "token": "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
    "interval": 2592000,
    "gracePeriod": 86400,
    "retryLimit": 3
  }'
```

---

## Webhook Events

All webhook payloads are signed with HMAC-SHA256. Verify the `x-webhook-signature` header before processing.

```typescript
// Verify signature
import { createHmac } from 'crypto';

function verifyWebhook(payload: string, signature: string, secret: string): boolean {
  const expected = createHmac('sha256', secret)
    .update(payload)
    .digest('hex');
  return expected === signature;
}
```

**Event payloads:**

```json
// Payment succeeded
{
  "event": "subscription.payment.success",
  "subscriptionId": "abc123",
  "amount": "100000000",
  "token": "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
  "timestamp": 1747339200
}

// Payment failed (retrying)
{
  "event": "subscription.payment.failed",
  "subscriptionId": "abc123",
  "retries": 2,
  "timestamp": 1747339200
}

// Subscription cancelled
{
  "event": "subscription.cancelled",
  "subscriptionId": "abc123",
  "timestamp": 1747339200
}
```

---

## Getting Started

### Prerequisites

| Tool | Version |
|------|---------|
| Node.js | 20+ |
| pnpm | 10+ |
| Rust | stable |
| Docker + Compose | latest |
| [Stellar CLI](https://developers.stellar.org/docs/tools/developer-tools/cli/install-cli) | latest |

### 1. Clone & Install

```bash
git clone https://github.com/brite-side0/SS-Billing.git
cd SS-Billing
pnpm install
```

### 2. Configure Environment

```bash
cp .env.example .env
```

```env
# .env
DATABASE_URL=postgresql://postgres:postgres@localhost:5432/ss_billing
REDIS_URL=redis://localhost:6379

# Fill these after deploying the contract
CONTRACT_ID=CXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
SIGNER_SECRET=SXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX

JWT_SECRET=your-jwt-secret
WEBHOOK_SECRET=your-webhook-secret

STELLAR_NETWORK=testnet
STELLAR_RPC_URL=https://soroban-testnet.stellar.org
STELLAR_HORIZON_URL=https://horizon-testnet.stellar.org
```

### 3. Deploy the Soroban Contract

```bash
# Install Rust wasm target
rustup target add wasm32-unknown-unknown

# Build the contract
cd contracts/subscription-engine
cargo build --target wasm32-unknown-unknown --release

# Deploy to testnet
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/subscription_engine.wasm \
  --network testnet \
  --source <YOUR_SECRET_KEY>

# Copy the returned contract ID into your .env CONTRACT_ID
```

### 4. Start Infrastructure

```bash
cd infra/docker && docker compose up -d postgres redis
```

### 5. Run the API

```bash
pnpm --filter @ss-billing/api dev
# API available at http://localhost:3001
# Swagger docs at http://localhost:3001/docs
```

### 6. Run the Frontend

```bash
pnpm --filter @ss-billing/web dev
# App available at http://localhost:3000
```

### Full Stack (Docker)

```bash
cd infra/docker && docker compose up
```

---

## Testing

```bash
# Soroban contract tests
cd contracts/subscription-engine
cargo test

# API unit + integration tests
pnpm --filter @ss-billing/api test

# API test coverage
pnpm --filter @ss-billing/api test:cov
```

---

## Individual Repositories

This monorepo is also available as standalone repositories:

| Repo | Description |
|------|-------------|
| [SS-Billing-Frontend](https://github.com/brite-side0/SS-Billing-frontend) | Next.js 14 merchant & subscriber dashboards |
| [SS-Billing-Backend](https://github.com/brite-side0/SS-Billing-backend) | NestJS API, scheduler, indexer, webhooks |
| [SS-Billing-Contract](https://github.com/brite-side0/SS-Billing-contract) | Soroban smart contract (Rust) |

---

## Stellar Network

SS-Billing is built exclusively on the **Stellar network** using **Soroban** — Stellar's smart contract platform.

- **Testnet RPC:** `https://soroban-testnet.stellar.org`
- **Mainnet RPC:** `https://soroban-mainnet.stellar.org`
- **Explorer:** [stellar.expert](https://stellar.expert)
- **Token standard:** SEP-41 (compatible with USDC, XLM, and any Stellar asset)
- **Amounts:** Denominated in stroops — `1 XLM = 10,000,000 stroops`

---

## License

MIT © [brite-side0](https://github.com/brite-side0)
