use crate::api::{
    ClaimRewardsMsg, MinimumEligibleWeightResponse, UpdateMinimumEligibleWeightMsg,
    UpdateUserWeightsMsg, UserRewardsParams, UserRewardsResponse, UserWeight,
};
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Uint128;
use cw20::Cw20ReceiveMsg;

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: String,
    pub enterprise_contract: String,
    pub initial_weights: Vec<UserWeight>,
    /// Optional minimum weight that the user must have to be eligible for rewards distributions
    pub minimum_eligible_weight: Option<Uint128>,
}

#[cw_serde]
pub enum ExecuteMsg {
    UpdateUserWeights(UpdateUserWeightsMsg),
    UpdateMinimumEligibleWeight(UpdateMinimumEligibleWeightMsg),
    DistributeNative {},
    ClaimRewards(ClaimRewardsMsg),
    Receive(Cw20ReceiveMsg),
}

#[cw_serde]
pub enum Cw20HookMsg {
    Distribute {},
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(UserRewardsResponse)]
    UserRewards(UserRewardsParams),
    #[returns(MinimumEligibleWeightResponse)]
    MinimumEligibleWeight {},
}

#[cw_serde]
pub struct MigrateMsg {
    pub new_admin: String,
    pub new_enterprise_contract: String,
}
