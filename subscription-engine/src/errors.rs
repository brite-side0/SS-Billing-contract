use soroban_sdk::contracterror;

#[contracterror]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ContractError {
    MerchantNotFound = 1,
    MerchantAlreadyExists = 2,
    MerchantInactive = 3,
    PlanNotFound = 4,
    PlanInactive = 5,
    SubscriptionNotFound = 6,
    SubscriptionNotActive = 7,
    SubscriptionAlreadyExists = 8,
    BillingNotDue = 9,
    InsufficientBalance = 10,
    RetryLimitExceeded = 11,
    Unauthorized = 12,
    InvalidAmount = 13,
    InvalidInterval = 14,
    AlreadyPaused = 15,
    NotPaused = 16,
    AlreadyCancelled = 17,
}
