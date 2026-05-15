use soroban_sdk::{contracttype, Address, Symbol};

// ── Enums ────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SubscriptionStatus {
    Active,
    Paused,
    Cancelled,
    GracePeriod,
    Failed,
}

// ── Core Structs ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub struct Merchant {
    pub merchant_id: Address,
    pub name: Symbol,
    pub treasury_wallet: Address,
    pub active: bool,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SubscriptionPlan {
    pub plan_id: u64,
    pub merchant_id: Address,
    pub name: Symbol,
    pub amount: i128,
    pub token: Address,
    pub interval: u64,   // seconds between billing cycles
    pub grace_period: u64,
    pub retry_limit: u32,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Subscriber {
    pub subscriber: Address,
    pub plan_id: u64,
    pub next_billing_at: u64,
    pub status: SubscriptionStatus,
    pub retries: u32,
    pub started_at: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PaymentRecord {
    pub payment_id: u64,
    pub subscriber: Address,
    pub merchant: Address,
    pub amount: i128,
    pub timestamp: u64,
    pub success: bool,
}

// ── Storage Keys ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Merchant(Address),
    Plan(u64),
    Subscriber(Address, u64),
    Payment(u64),
    MerchantPlans(Address),
    SubscriberPlans(Address),
    PlanCounter,
    PaymentCounter,
}
