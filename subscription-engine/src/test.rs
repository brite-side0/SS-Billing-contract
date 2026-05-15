#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Symbol,
};

use crate::{SubscriptionEngine, SubscriptionEngineClient};
use crate::types::SubscriptionStatus;
use crate::errors::ContractError;

const INTERVAL: u64 = 2_592_000; // 30 days in seconds
const GRACE: u64 = 86_400;       // 1 day
const AMOUNT: i128 = 100_0000000; // 100 tokens (7 decimals)

fn setup() -> (Env, SubscriptionEngineClient<'static>, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SubscriptionEngine);
    let client = SubscriptionEngineClient::new(&env, &contract_id);

    let merchant = Address::generate(&env);
    let subscriber = Address::generate(&env);

    // Deploy a test token
    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin.clone()).address();
    let token_admin_client = StellarAssetClient::new(&env, &token_id);

    // Mint tokens to subscriber
    token_admin_client.mint(&subscriber, &(AMOUNT * 10));

    (env, client, merchant, subscriber, token_id, token_admin)
}

fn register_and_plan(
    client: &SubscriptionEngineClient,
    merchant: &Address,
    token: &Address,
) -> u64 {
    client.register_merchant(&Symbol::new(&client.env, "AcmeCorp"), merchant);
    client.create_plan(
        merchant,
        &Symbol::new(&client.env, "Pro"),
        &AMOUNT,
        token,
        &INTERVAL,
        &GRACE,
        &3u32,
    )
}

// ── Merchant Tests ────────────────────────────────────────────────────────────

#[test]
fn test_register_merchant() {
    let (env, client, merchant, _, _, _) = setup();
    client.register_merchant(&Symbol::new(&env, "Acme"), &merchant);
    let m = client.get_merchant(&merchant).unwrap();
    assert_eq!(m.active, true);
    assert_eq!(m.treasury_wallet, merchant);
}

#[test]
fn test_register_merchant_duplicate_fails() {
    let (env, client, merchant, _, _, _) = setup();
    client.register_merchant(&Symbol::new(&env, "Acme"), &merchant);
    let result = client.try_register_merchant(&Symbol::new(&env, "Acme"), &merchant);
    assert_eq!(result, Err(Ok(ContractError::MerchantAlreadyExists)));
}

#[test]
fn test_update_treasury() {
    let (env, client, merchant, _, _, _) = setup();
    client.register_merchant(&Symbol::new(&env, "Acme"), &merchant);
    let new_treasury = Address::generate(&env);
    client.update_treasury(&merchant, &new_treasury);
    let m = client.get_merchant(&merchant).unwrap();
    assert_eq!(m.treasury_wallet, new_treasury);
}

// ── Plan Tests ────────────────────────────────────────────────────────────────

#[test]
fn test_create_plan() {
    let (env, client, merchant, _, token, _) = setup();
    let plan_id = register_and_plan(&client, &merchant, &token);
    assert_eq!(plan_id, 1);
    let plan = client.get_plan(&plan_id).unwrap();
    assert_eq!(plan.amount, AMOUNT);
    assert_eq!(plan.active, true);
}

#[test]
fn test_create_plan_invalid_amount_fails() {
    let (env, client, merchant, _, token, _) = setup();
    client.register_merchant(&Symbol::new(&env, "Acme"), &merchant);
    let result = client.try_create_plan(
        &merchant,
        &Symbol::new(&env, "Bad"),
        &0i128,
        &token,
        &INTERVAL,
        &GRACE,
        &3u32,
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidAmount)));
}

#[test]
fn test_disable_plan() {
    let (env, client, merchant, _, token, _) = setup();
    let plan_id = register_and_plan(&client, &merchant, &token);
    client.disable_plan(&merchant, &plan_id);
    let plan = client.get_plan(&plan_id).unwrap();
    assert_eq!(plan.active, false);
}

// ── Subscription Tests ────────────────────────────────────────────────────────

#[test]
fn test_subscribe_and_first_payment() {
    let (env, client, merchant, subscriber, token, _) = setup();
    let plan_id = register_and_plan(&client, &merchant, &token);
    client.subscribe(&subscriber, &plan_id);
    let sub = client.get_subscriber(&subscriber, &plan_id).unwrap();
    assert_eq!(sub.status, SubscriptionStatus::Active);
    assert_eq!(sub.next_billing_at, INTERVAL); // ledger starts at 0
}

#[test]
fn test_subscribe_to_inactive_plan_fails() {
    let (env, client, merchant, subscriber, token, _) = setup();
    let plan_id = register_and_plan(&client, &merchant, &token);
    client.disable_plan(&merchant, &plan_id);
    let result = client.try_subscribe(&subscriber, &plan_id);
    assert_eq!(result, Err(Ok(ContractError::PlanInactive)));
}

#[test]
fn test_subscribe_duplicate_fails() {
    let (_, client, merchant, subscriber, token, _) = setup();
    let plan_id = register_and_plan(&client, &merchant, &token);
    client.subscribe(&subscriber, &plan_id);
    let result = client.try_subscribe(&subscriber, &plan_id);
    assert_eq!(result, Err(Ok(ContractError::SubscriptionAlreadyExists)));
}

// ── Billing Tests ─────────────────────────────────────────────────────────────

#[test]
fn test_process_payment_success() {
    let (env, client, merchant, subscriber, token, _) = setup();
    let plan_id = register_and_plan(&client, &merchant, &token);
    client.subscribe(&subscriber, &plan_id);

    // Advance ledger past billing date
    env.ledger().set(LedgerInfo {
        timestamp: INTERVAL + 1,
        protocol_version: 21,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 1,
        min_persistent_entry_ttl: 1,
        max_entry_ttl: 6_312_000,
    });

    client.process_payment(&subscriber, &plan_id);
    let sub = client.get_subscriber(&subscriber, &plan_id).unwrap();
    assert_eq!(sub.status, SubscriptionStatus::Active);
    assert_eq!(sub.retries, 0);
}

#[test]
fn test_process_payment_not_due_fails() {
    let (_, client, merchant, subscriber, token, _) = setup();
    let plan_id = register_and_plan(&client, &merchant, &token);
    client.subscribe(&subscriber, &plan_id);
    // Don't advance ledger — billing not due
    let result = client.try_process_payment(&subscriber, &plan_id);
    assert_eq!(result, Err(Ok(ContractError::BillingNotDue)));
}

#[test]
fn test_process_payment_insufficient_balance() {
    let (env, client, merchant, subscriber, token, token_admin) = setup();
    let plan_id = register_and_plan(&client, &merchant, &token);
    client.subscribe(&subscriber, &plan_id);

    // Drain subscriber balance
    let token_client = TokenClient::new(&env, &token);
    let balance = token_client.balance(&subscriber);
    let treasury = client.get_merchant(&merchant).unwrap().treasury_wallet;
    // Transfer all remaining tokens away via admin burn — use transfer to a burn address
    let burn_addr = Address::generate(&env);
    StellarAssetClient::new(&env, &token).set_authorized(&subscriber, &false);
    // Simpler: just set balance to 0 via admin
    StellarAssetClient::new(&env, &token).mint(&burn_addr, &1); // ensure admin works
    // Clawback all subscriber tokens
    StellarAssetClient::new(&env, &token).clawback(&subscriber, &balance);

    env.ledger().set(LedgerInfo {
        timestamp: INTERVAL + 1,
        protocol_version: 21,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 1,
        min_persistent_entry_ttl: 1,
        max_entry_ttl: 6_312_000,
    });

    let result = client.try_process_payment(&subscriber, &plan_id);
    assert_eq!(result, Err(Ok(ContractError::InsufficientBalance)));
    let sub = client.get_subscriber(&subscriber, &plan_id).unwrap();
    assert_eq!(sub.status, SubscriptionStatus::GracePeriod);
    assert_eq!(sub.retries, 1);
}

#[test]
fn test_retry_limit_exhaustion() {
    let (env, client, merchant, subscriber, token, _) = setup();
    // Create plan with retry_limit = 1
    client.register_merchant(&Symbol::new(&env, "Acme"), &merchant);
    let plan_id = client.create_plan(
        &merchant,
        &Symbol::new(&env, "Pro"),
        &AMOUNT,
        &token,
        &INTERVAL,
        &GRACE,
        &1u32,
    );
    client.subscribe(&subscriber, &plan_id);

    // Drain balance
    let balance = TokenClient::new(&env, &token).balance(&subscriber);
    StellarAssetClient::new(&env, &token).clawback(&subscriber, &balance);

    env.ledger().set(LedgerInfo {
        timestamp: INTERVAL + 1,
        protocol_version: 21,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 1,
        min_persistent_entry_ttl: 1,
        max_entry_ttl: 6_312_000,
    });

    // First attempt → GracePeriod
    let _ = client.try_process_payment(&subscriber, &plan_id);
    // Second attempt → RetryLimitExceeded
    let result = client.try_process_payment(&subscriber, &plan_id);
    assert_eq!(result, Err(Ok(ContractError::RetryLimitExceeded)));
    let sub = client.get_subscriber(&subscriber, &plan_id).unwrap();
    assert_eq!(sub.status, SubscriptionStatus::Failed);
}

// ── Pause / Resume / Cancel Tests ────────────────────────────────────────────

#[test]
fn test_pause_and_resume() {
    let (env, client, merchant, subscriber, token, _) = setup();
    let plan_id = register_and_plan(&client, &merchant, &token);
    client.subscribe(&subscriber, &plan_id);

    client.pause_subscription(&subscriber, &plan_id);
    let sub = client.get_subscriber(&subscriber, &plan_id).unwrap();
    assert_eq!(sub.status, SubscriptionStatus::Paused);

    // Billing should fail while paused
    env.ledger().set(LedgerInfo {
        timestamp: INTERVAL + 1,
        protocol_version: 21,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 1,
        min_persistent_entry_ttl: 1,
        max_entry_ttl: 6_312_000,
    });
    let result = client.try_process_payment(&subscriber, &plan_id);
    assert_eq!(result, Err(Ok(ContractError::SubscriptionNotActive)));

    client.resume_subscription(&subscriber, &plan_id);
    let sub = client.get_subscriber(&subscriber, &plan_id).unwrap();
    assert_eq!(sub.status, SubscriptionStatus::Active);
}

#[test]
fn test_cancel_subscription() {
    let (_, client, merchant, subscriber, token, _) = setup();
    let plan_id = register_and_plan(&client, &merchant, &token);
    client.subscribe(&subscriber, &plan_id);
    client.cancel_subscription(&subscriber, &plan_id);
    let sub = client.get_subscriber(&subscriber, &plan_id).unwrap();
    assert_eq!(sub.status, SubscriptionStatus::Cancelled);

    // Cannot cancel again
    let result = client.try_cancel_subscription(&subscriber, &plan_id);
    assert_eq!(result, Err(Ok(ContractError::AlreadyCancelled)));
}

#[test]
fn test_cancelled_subscription_cannot_be_billed() {
    let (env, client, merchant, subscriber, token, _) = setup();
    let plan_id = register_and_plan(&client, &merchant, &token);
    client.subscribe(&subscriber, &plan_id);
    client.cancel_subscription(&subscriber, &plan_id);

    env.ledger().set(LedgerInfo {
        timestamp: INTERVAL + 1,
        protocol_version: 21,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 1,
        min_persistent_entry_ttl: 1,
        max_entry_ttl: 6_312_000,
    });
    let result = client.try_process_payment(&subscriber, &plan_id);
    assert_eq!(result, Err(Ok(ContractError::AlreadyCancelled)));
}

// ── Query Tests ───────────────────────────────────────────────────────────────

#[test]
fn test_get_merchant_plans() {
    let (env, client, merchant, _, token, _) = setup();
    client.register_merchant(&Symbol::new(&env, "Acme"), &merchant);
    client.create_plan(&merchant, &Symbol::new(&env, "Basic"), &AMOUNT, &token, &INTERVAL, &GRACE, &3u32);
    client.create_plan(&merchant, &Symbol::new(&env, "Pro"), &(AMOUNT * 2), &token, &INTERVAL, &GRACE, &3u32);
    let plans = client.get_merchant_plans(&merchant);
    assert_eq!(plans.len(), 2);
}

#[test]
fn test_get_subscriber_plans() {
    let (env, client, merchant, subscriber, token, _) = setup();
    client.register_merchant(&Symbol::new(&env, "Acme"), &merchant);
    let p1 = client.create_plan(&merchant, &Symbol::new(&env, "Basic"), &AMOUNT, &token, &INTERVAL, &GRACE, &3u32);
    let p2 = client.create_plan(&merchant, &Symbol::new(&env, "Pro"), &(AMOUNT * 2), &token, &INTERVAL, &GRACE, &3u32);
    client.subscribe(&subscriber, &p1);
    client.subscribe(&subscriber, &p2);
    let plans = client.get_subscriber_plans(&subscriber);
    assert_eq!(plans.len(), 2);
}
