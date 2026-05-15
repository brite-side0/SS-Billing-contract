#![no_std]

mod errors;
mod events;
mod storage;
mod types;

#[cfg(test)]
mod test;

use errors::ContractError;
use soroban_sdk::{
    contract, contractimpl, token, Address, Env, Symbol, Vec,
};
use storage::*;
use types::*;

#[contract]
pub struct SubscriptionEngine;

#[contractimpl]
impl SubscriptionEngine {
    // ── Merchant Functions ────────────────────────────────────────────────────

    /// Register a new merchant. Caller becomes the merchant_id.
    pub fn register_merchant(
        env: Env,
        name: Symbol,
        treasury_wallet: Address,
    ) -> Result<(), ContractError> {
        let merchant_id = env.current_contract_address();
        // In practice the invoker is the merchant; require their auth
        // We use the treasury_wallet as the authenticated merchant identity
        treasury_wallet.require_auth();

        if load_merchant(&env, &treasury_wallet).is_some() {
            return Err(ContractError::MerchantAlreadyExists);
        }

        let merchant = Merchant {
            merchant_id: treasury_wallet.clone(),
            name,
            treasury_wallet: treasury_wallet.clone(),
            active: true,
            created_at: env.ledger().timestamp(),
        };
        save_merchant(&env, &merchant);
        events::merchant_registered(&env, &treasury_wallet);
        Ok(())
    }

    /// Update treasury wallet for a merchant.
    pub fn update_treasury(
        env: Env,
        merchant_id: Address,
        new_treasury: Address,
    ) -> Result<(), ContractError> {
        merchant_id.require_auth();
        let mut merchant = load_merchant(&env, &merchant_id)
            .ok_or(ContractError::MerchantNotFound)?;
        merchant.treasury_wallet = new_treasury;
        save_merchant(&env, &merchant);
        Ok(())
    }

    // ── Plan Functions ────────────────────────────────────────────────────────

    /// Create a subscription plan under a merchant.
    pub fn create_plan(
        env: Env,
        merchant_id: Address,
        name: Symbol,
        amount: i128,
        token: Address,
        interval: u64,
        grace_period: u64,
        retry_limit: u32,
    ) -> Result<u64, ContractError> {
        merchant_id.require_auth();

        let merchant = load_merchant(&env, &merchant_id)
            .ok_or(ContractError::MerchantNotFound)?;
        if !merchant.active {
            return Err(ContractError::MerchantInactive);
        }
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
        if interval == 0 {
            return Err(ContractError::InvalidInterval);
        }

        let plan_id = next_plan_id(&env);
        let plan = SubscriptionPlan {
            plan_id,
            merchant_id: merchant_id.clone(),
            name,
            amount,
            token,
            interval,
            grace_period,
            retry_limit,
            active: true,
        };
        save_plan(&env, &plan);
        add_merchant_plan(&env, &merchant_id, plan_id);
        events::plan_created(&env, &merchant_id, plan_id);
        Ok(plan_id)
    }

    /// Update mutable plan fields (amount, interval, grace_period, retry_limit).
    pub fn update_plan(
        env: Env,
        merchant_id: Address,
        plan_id: u64,
        amount: i128,
        interval: u64,
        grace_period: u64,
        retry_limit: u32,
    ) -> Result<(), ContractError> {
        merchant_id.require_auth();
        let mut plan = load_plan(&env, plan_id).ok_or(ContractError::PlanNotFound)?;
        if plan.merchant_id != merchant_id {
            return Err(ContractError::Unauthorized);
        }
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
        if interval == 0 {
            return Err(ContractError::InvalidInterval);
        }
        plan.amount = amount;
        plan.interval = interval;
        plan.grace_period = grace_period;
        plan.retry_limit = retry_limit;
        save_plan(&env, &plan);
        Ok(())
    }

    /// Disable a plan (no new subscriptions, existing ones continue until cancelled).
    pub fn disable_plan(
        env: Env,
        merchant_id: Address,
        plan_id: u64,
    ) -> Result<(), ContractError> {
        merchant_id.require_auth();
        let mut plan = load_plan(&env, plan_id).ok_or(ContractError::PlanNotFound)?;
        if plan.merchant_id != merchant_id {
            return Err(ContractError::Unauthorized);
        }
        plan.active = false;
        save_plan(&env, &plan);
        Ok(())
    }

    // ── Subscription Functions ────────────────────────────────────────────────

    /// Subscribe to a plan. Subscriber authorizes once; billing is delegated.
    pub fn subscribe(
        env: Env,
        subscriber: Address,
        plan_id: u64,
    ) -> Result<(), ContractError> {
        subscriber.require_auth();

        let plan = load_plan(&env, plan_id).ok_or(ContractError::PlanNotFound)?;
        if !plan.active {
            return Err(ContractError::PlanInactive);
        }

        let merchant = load_merchant(&env, &plan.merchant_id)
            .ok_or(ContractError::MerchantNotFound)?;
        if !merchant.active {
            return Err(ContractError::MerchantInactive);
        }

        if load_subscriber(&env, &subscriber, plan_id).is_some() {
            return Err(ContractError::SubscriptionAlreadyExists);
        }

        let now = env.ledger().timestamp();
        // Charge first payment immediately
        Self::_transfer_payment(&env, &subscriber, &merchant.treasury_wallet, &plan.token, plan.amount)?;

        let record_id = next_payment_id(&env);
        save_payment(&env, &PaymentRecord {
            payment_id: record_id,
            subscriber: subscriber.clone(),
            merchant: plan.merchant_id.clone(),
            amount: plan.amount,
            timestamp: now,
            success: true,
        });

        let next_billing_at = now + plan.interval;
        let sub = Subscriber {
            subscriber: subscriber.clone(),
            plan_id,
            next_billing_at,
            status: SubscriptionStatus::Active,
            retries: 0,
            started_at: now,
        };
        save_subscriber(&env, &sub);
        add_subscriber_plan(&env, &subscriber, plan_id);
        events::subscribed(&env, &subscriber, plan_id, next_billing_at);
        events::payment_success(&env, &subscriber, plan.amount, now);
        Ok(())
    }

    /// Process a recurring billing cycle. Can be called by anyone (scheduler/keeper).
    pub fn process_payment(
        env: Env,
        subscriber: Address,
        plan_id: u64,
    ) -> Result<(), ContractError> {
        let mut sub = load_subscriber(&env, &subscriber, plan_id)
            .ok_or(ContractError::SubscriptionNotFound)?;

        match sub.status {
            SubscriptionStatus::Cancelled => return Err(ContractError::AlreadyCancelled),
            SubscriptionStatus::Paused => return Err(ContractError::SubscriptionNotActive),
            _ => {}
        }

        let now = env.ledger().timestamp();
        let plan = load_plan(&env, plan_id).ok_or(ContractError::PlanNotFound)?;
        let merchant = load_merchant(&env, &plan.merchant_id)
            .ok_or(ContractError::MerchantNotFound)?;

        // Billing not yet due (allow grace period window)
        if now < sub.next_billing_at {
            return Err(ContractError::BillingNotDue);
        }

        // Check if in grace period
        let in_grace = now > sub.next_billing_at + plan.grace_period;

        match Self::_transfer_payment(&env, &subscriber, &merchant.treasury_wallet, &plan.token, plan.amount) {
            Ok(_) => {
                let record_id = next_payment_id(&env);
                save_payment(&env, &PaymentRecord {
                    payment_id: record_id,
                    subscriber: subscriber.clone(),
                    merchant: plan.merchant_id.clone(),
                    amount: plan.amount,
                    timestamp: now,
                    success: true,
                });
                sub.next_billing_at = now + plan.interval;
                sub.retries = 0;
                sub.status = SubscriptionStatus::Active;
                save_subscriber(&env, &sub);
                events::payment_success(&env, &subscriber, plan.amount, now);
            }
            Err(_) => {
                sub.retries += 1;
                events::retry_attempted(&env, &subscriber, plan_id, sub.retries);

                if sub.retries >= plan.retry_limit {
                    sub.status = SubscriptionStatus::Failed;
                    save_subscriber(&env, &sub);
                    let record_id = next_payment_id(&env);
                    save_payment(&env, &PaymentRecord {
                        payment_id: record_id,
                        subscriber: subscriber.clone(),
                        merchant: plan.merchant_id.clone(),
                        amount: plan.amount,
                        timestamp: now,
                        success: false,
                    });
                    events::payment_failed(&env, &subscriber, plan_id, sub.retries);
                    return Err(ContractError::RetryLimitExceeded);
                } else {
                    sub.status = SubscriptionStatus::GracePeriod;
                    save_subscriber(&env, &sub);
                    events::payment_failed(&env, &subscriber, plan_id, sub.retries);
                    return Err(ContractError::InsufficientBalance);
                }
            }
        }
        Ok(())
    }

    /// Pause an active subscription.
    pub fn pause_subscription(
        env: Env,
        subscriber: Address,
        plan_id: u64,
    ) -> Result<(), ContractError> {
        subscriber.require_auth();
        let mut sub = load_subscriber(&env, &subscriber, plan_id)
            .ok_or(ContractError::SubscriptionNotFound)?;
        if sub.status == SubscriptionStatus::Paused {
            return Err(ContractError::AlreadyPaused);
        }
        if sub.status == SubscriptionStatus::Cancelled {
            return Err(ContractError::AlreadyCancelled);
        }
        sub.status = SubscriptionStatus::Paused;
        save_subscriber(&env, &sub);
        events::subscription_paused(&env, &subscriber, plan_id);
        Ok(())
    }

    /// Resume a paused subscription.
    pub fn resume_subscription(
        env: Env,
        subscriber: Address,
        plan_id: u64,
    ) -> Result<(), ContractError> {
        subscriber.require_auth();
        let mut sub = load_subscriber(&env, &subscriber, plan_id)
            .ok_or(ContractError::SubscriptionNotFound)?;
        if sub.status != SubscriptionStatus::Paused {
            return Err(ContractError::NotPaused);
        }
        // Reset next billing to now + interval on resume
        sub.next_billing_at = env.ledger().timestamp() + load_plan(&env, plan_id)
            .ok_or(ContractError::PlanNotFound)?.interval;
        sub.status = SubscriptionStatus::Active;
        save_subscriber(&env, &sub);
        events::subscription_resumed(&env, &subscriber, plan_id);
        Ok(())
    }

    /// Cancel a subscription permanently.
    pub fn cancel_subscription(
        env: Env,
        subscriber: Address,
        plan_id: u64,
    ) -> Result<(), ContractError> {
        subscriber.require_auth();
        let mut sub = load_subscriber(&env, &subscriber, plan_id)
            .ok_or(ContractError::SubscriptionNotFound)?;
        if sub.status == SubscriptionStatus::Cancelled {
            return Err(ContractError::AlreadyCancelled);
        }
        sub.status = SubscriptionStatus::Cancelled;
        save_subscriber(&env, &sub);
        events::subscription_cancelled(&env, &subscriber, plan_id);
        Ok(())
    }

    // ── Query Functions ───────────────────────────────────────────────────────

    pub fn get_merchant(env: Env, merchant_id: Address) -> Option<Merchant> {
        load_merchant(&env, &merchant_id)
    }

    pub fn get_plan(env: Env, plan_id: u64) -> Option<SubscriptionPlan> {
        load_plan(&env, plan_id)
    }

    pub fn get_subscriber(env: Env, subscriber: Address, plan_id: u64) -> Option<Subscriber> {
        load_subscriber(&env, &subscriber, plan_id)
    }

    pub fn get_payment(env: Env, payment_id: u64) -> Option<PaymentRecord> {
        load_payment(&env, payment_id)
    }

    pub fn get_merchant_plans(env: Env, merchant_id: Address) -> Vec<u64> {
        storage::get_merchant_plans(&env, &merchant_id)
    }

    pub fn get_subscriber_plans(env: Env, subscriber: Address) -> Vec<u64> {
        storage::get_subscriber_plans(&env, &subscriber)
    }

    // ── Internal Helpers ──────────────────────────────────────────────────────

    fn _transfer_payment(
        env: &Env,
        from: &Address,
        to: &Address,
        token: &Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        let client = token::Client::new(env, token);
        let balance = client.balance(from);
        if balance < amount {
            return Err(ContractError::InsufficientBalance);
        }
        client.transfer(from, to, &amount);
        Ok(())
    }
}
