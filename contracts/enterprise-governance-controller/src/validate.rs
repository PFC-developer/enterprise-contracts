use crate::state::GOV_CONFIG;
use cosmwasm_std::{Addr, CosmosMsg, Decimal, Deps, StdError, Uint128};
use cw_asset::{AssetInfo, AssetInfoBase};
use cw_utils::Duration;
use enterprise_governance_controller_api::api::ModifyValue::Change;
use enterprise_governance_controller_api::api::ProposalAction::{
    DistributeFunds, ExecuteMsgs, ModifyMultisigMembership, RequestFundingFromDao,
    UpdateAssetWhitelist, UpdateCouncil, UpdateGovConfig, UpdateMetadata,
    UpdateMinimumWeightForRewards, UpdateNftWhitelist, UpgradeDao,
};
use enterprise_governance_controller_api::api::{
    DaoCouncil, DaoCouncilSpec, DistributeFundsMsg, ExecuteMsgsMsg, GovConfig,
    ModifyMultisigMembershipMsg, ProposalAction, ProposalActionType, ProposalDeposit,
    UpdateGovConfigMsg,
};
use enterprise_governance_controller_api::error::GovernanceControllerError::{
    DuplicateCouncilMember, InsufficientProposalDeposit, InvalidArgument, InvalidCosmosMessage,
    Std, UnsupportedCouncilProposalAction, ZeroVoteDuration,
};
use enterprise_governance_controller_api::error::{
    GovernanceControllerError, GovernanceControllerResult,
};
use enterprise_protocol::api::DaoType::{Multisig, Nft};
use enterprise_protocol::api::{DaoType, UpgradeDaoMsg};
use std::collections::{HashMap, HashSet};
use GovernanceControllerError::{
    MinimumDepositNotAllowed, UnsupportedOperationForDaoType, VoteDurationLongerThanUnstaking,
};

pub fn validate_dao_gov_config(
    dao_type: &DaoType,
    dao_gov_config: &GovConfig,
) -> GovernanceControllerResult<()> {
    if dao_gov_config.vote_duration == 0 {
        return Err(ZeroVoteDuration);
    }

    if let Duration::Time(unlocking_time) = dao_gov_config.unlocking_period {
        if unlocking_time < dao_gov_config.vote_duration {
            return Err(VoteDurationLongerThanUnstaking {});
        }
    }

    validate_quorum_value(dao_gov_config.quorum)?;

    validate_threshold_value(dao_gov_config.threshold)?;

    if let Some(veto_threshold) = dao_gov_config.veto_threshold {
        if veto_threshold > Decimal::one() || veto_threshold == Decimal::zero() {
            return Err(InvalidArgument {
                msg: "Invalid veto threshold, must be 0 < threshold <= 1".to_string(),
            });
        }
    }

    if dao_gov_config.minimum_deposit.is_some() && (dao_type == &Nft || dao_type == &Multisig) {
        return Err(MinimumDepositNotAllowed {});
    }

    Ok(())
}

fn validate_quorum_value(quorum: Decimal) -> GovernanceControllerResult<()> {
    if quorum > Decimal::one() || quorum == Decimal::zero() {
        return Err(InvalidArgument {
            msg: "Invalid quorum, must be 0 < quorum <= 1".to_string(),
        });
    }
    Ok(())
}

fn validate_threshold_value(threshold: Decimal) -> GovernanceControllerResult<()> {
    if threshold > Decimal::one() || threshold == Decimal::zero() {
        return Err(InvalidArgument {
            msg: "Invalid threshold, must be 0 < threshold <= 1".to_string(),
        });
    }

    Ok(())
}

pub fn validate_deposit(
    gov_config: &GovConfig,
    deposit: &Option<ProposalDeposit>,
) -> GovernanceControllerResult<()> {
    match gov_config.minimum_deposit {
        None => Ok(()),
        Some(required_amount) => {
            let deposited_amount = deposit
                .as_ref()
                .map(|deposit| deposit.amount)
                .unwrap_or_default();

            if deposited_amount >= required_amount {
                Ok(())
            } else {
                Err(InsufficientProposalDeposit { required_amount })
            }
        }
    }
}

pub fn validate_proposal_actions(
    deps: Deps,
    dao_type: DaoType,
    proposal_actions: &Vec<ProposalAction>,
) -> GovernanceControllerResult<()> {
    for proposal_action in proposal_actions {
        match proposal_action {
            UpdateAssetWhitelist(msg) => {
                validate_asset_whitelist_changes(deps, &msg.add, &msg.remove)?
            }
            UpdateNftWhitelist(msg) => validate_nft_whitelist_changes(deps, &msg.add, &msg.remove)?,
            UpgradeDao(msg) => validate_upgrade_dao(deps, msg)?,
            ExecuteMsgs(msg) => validate_execute_msgs(msg)?,
            ModifyMultisigMembership(msg) => {
                validate_modify_multisig_membership(deps, dao_type.clone(), msg)?
            }
            UpdateCouncil(msg) => {
                validate_dao_council(deps, msg.dao_council.clone())?;
            }
            DistributeFunds(msg) => validate_distribute_funds(msg)?,
            UpdateGovConfig(msg) => {
                let gov_config = GOV_CONFIG.load(deps.storage)?;

                let updated_gov_config = apply_gov_config_changes(gov_config, msg);

                validate_dao_gov_config(&dao_type, &updated_gov_config)?;
            }
            UpdateMetadata(_) | RequestFundingFromDao(_) | UpdateMinimumWeightForRewards(_) => {
                // no-op
            }
        }
    }

    Ok(())
}

pub fn apply_gov_config_changes(gov_config: GovConfig, msg: &UpdateGovConfigMsg) -> GovConfig {
    let mut gov_config = gov_config;

    if let Change(quorum) = msg.quorum {
        gov_config.quorum = quorum;
    }

    if let Change(threshold) = msg.threshold {
        gov_config.threshold = threshold;
    }

    if let Change(veto_threshold) = msg.veto_threshold {
        gov_config.veto_threshold = veto_threshold;
    }

    if let Change(voting_duration) = msg.voting_duration {
        gov_config.vote_duration = voting_duration.u64();
    }

    if let Change(unlocking_period) = msg.unlocking_period {
        gov_config.unlocking_period = unlocking_period;
    }

    if let Change(minimum_deposit) = msg.minimum_deposit {
        gov_config.minimum_deposit = minimum_deposit;
    }

    if let Change(allow_early_proposal_execution) = msg.allow_early_proposal_execution {
        gov_config.allow_early_proposal_execution = allow_early_proposal_execution;
    }

    gov_config
}

pub fn normalize_asset_whitelist(
    deps: Deps,
    asset_whitelist: &Vec<AssetInfo>,
) -> GovernanceControllerResult<Vec<AssetInfo>> {
    let mut normalized_asset_whitelist: Vec<AssetInfo> = vec![];

    let asset_hashsets = split_asset_hashsets(deps, asset_whitelist)?;

    for denom in asset_hashsets.native {
        normalized_asset_whitelist.push(AssetInfo::native(denom))
    }

    for cw20 in asset_hashsets.cw20 {
        normalized_asset_whitelist.push(AssetInfo::cw20(cw20))
    }

    for (addr, token_id) in asset_hashsets.cw1155 {
        normalized_asset_whitelist.push(AssetInfo::cw1155(addr, token_id))
    }

    Ok(normalized_asset_whitelist)
}

fn validate_asset_whitelist_changes(
    deps: Deps,
    add: &Vec<AssetInfo>,
    remove: &Vec<AssetInfo>,
) -> GovernanceControllerResult<()> {
    let add_asset_hashsets = split_asset_hashsets(deps, add)?;
    let remove_asset_hashsets = split_asset_hashsets(deps, remove)?;

    if add_asset_hashsets
        .native
        .intersection(&remove_asset_hashsets.native)
        .count()
        > 0usize
    {
        return Err(GovernanceControllerError::AssetPresentInBothAddAndRemove);
    }
    if add_asset_hashsets
        .cw20
        .intersection(&remove_asset_hashsets.cw20)
        .count()
        > 0usize
    {
        return Err(GovernanceControllerError::AssetPresentInBothAddAndRemove);
    }
    if add_asset_hashsets
        .cw1155
        .intersection(&remove_asset_hashsets.cw1155)
        .count()
        > 0usize
    {
        return Err(GovernanceControllerError::AssetPresentInBothAddAndRemove);
    }

    Ok(())
}

fn split_asset_hashsets(
    deps: Deps,
    assets: &Vec<AssetInfo>,
) -> GovernanceControllerResult<AssetInfoHashSets> {
    let mut native_assets: HashSet<String> = HashSet::new();
    let mut cw20_assets: HashSet<Addr> = HashSet::new();
    let mut cw1155_assets: HashSet<(Addr, String)> = HashSet::new();
    for asset in assets {
        match asset {
            AssetInfo::Native(denom) => {
                if native_assets.contains(denom) {
                    return Err(GovernanceControllerError::DuplicateAssetFound);
                } else {
                    native_assets.insert(denom.clone());
                }
            }
            AssetInfo::Cw20(addr) => {
                let addr = deps.api.addr_validate(addr.as_ref())?;
                if cw20_assets.contains(&addr) {
                    return Err(GovernanceControllerError::DuplicateAssetFound);
                } else {
                    cw20_assets.insert(addr);
                }
            }
            AssetInfo::Cw1155(addr, id) => {
                let addr = deps.api.addr_validate(addr.as_ref())?;
                if cw1155_assets.contains(&(addr.clone(), id.to_string())) {
                    return Err(GovernanceControllerError::DuplicateAssetFound);
                } else {
                    cw1155_assets.insert((addr, id.to_string()));
                }
            }
            _ => {
                return Err(GovernanceControllerError::CustomError {
                    val: "Unsupported whitelist asset type".to_string(),
                })
            }
        }
    }

    Ok(AssetInfoHashSets {
        native: native_assets,
        cw20: cw20_assets,
        cw1155: cw1155_assets,
    })
}

struct AssetInfoHashSets {
    pub native: HashSet<String>,
    pub cw20: HashSet<Addr>,
    pub cw1155: HashSet<(Addr, String)>,
}

fn validate_nft_whitelist_changes(
    deps: Deps,
    add: &Vec<Addr>,
    remove: &Vec<Addr>,
) -> GovernanceControllerResult<()> {
    let mut add_nfts: HashSet<Addr> = HashSet::new();
    for nft in add {
        let nft = deps.api.addr_validate(nft.as_ref())?;
        if add_nfts.contains(&nft) {
            return Err(GovernanceControllerError::DuplicateNftFound);
        } else {
            add_nfts.insert(nft);
        }
    }

    let mut remove_nfts: HashSet<Addr> = HashSet::new();
    for nft in remove {
        let nft = deps.api.addr_validate(nft.as_ref())?;
        if remove_nfts.contains(&nft) {
            return Err(GovernanceControllerError::DuplicateNftFound);
        } else {
            remove_nfts.insert(nft);
        }
    }

    if add_nfts.intersection(&remove_nfts).count() > 0usize {
        return Err(GovernanceControllerError::NftPresentInBothAddAndRemove);
    }

    Ok(())
}

pub fn validate_upgrade_dao(_deps: Deps, _msg: &UpgradeDaoMsg) -> GovernanceControllerResult<()> {
    // TODO: we no longer need this, right? doesn't need to check for valid code IDs
    Ok(())
}

fn validate_execute_msgs(msg: &ExecuteMsgsMsg) -> GovernanceControllerResult<()> {
    for msg in msg.msgs.iter() {
        serde_json_wasm::from_str::<CosmosMsg>(msg.as_str()).map_err(|_| InvalidCosmosMessage)?;
    }
    Ok(())
}

pub fn validate_modify_multisig_membership(
    deps: Deps,
    dao_type: DaoType,
    msg: &ModifyMultisigMembershipMsg,
) -> GovernanceControllerResult<()> {
    if dao_type != Multisig {
        return Err(UnsupportedOperationForDaoType {
            dao_type: dao_type.to_string(),
        });
    }

    let mut deduped_addr_validated_members: HashMap<Addr, Uint128> = HashMap::new();

    for member in &msg.edit_members {
        let addr = deps.api.addr_validate(&member.user)?;

        if deduped_addr_validated_members
            .insert(addr, member.weight)
            .is_some()
        {
            return Err(GovernanceControllerError::DuplicateMultisigMemberWeightEdit);
        }
    }

    Ok(())
}

pub fn validate_dao_council(
    deps: Deps,
    dao_council: Option<DaoCouncilSpec>,
) -> GovernanceControllerResult<Option<DaoCouncil>> {
    match dao_council {
        None => Ok(None),
        Some(dao_council) => {
            let members = validate_no_duplicate_council_members(deps, dao_council.members)?;
            validate_allowed_council_proposal_types(
                dao_council.allowed_proposal_action_types.clone(),
            )?;

            validate_quorum_value(dao_council.quorum)?;
            validate_threshold_value(dao_council.threshold)?;

            Ok(Some(DaoCouncil {
                members,
                allowed_proposal_action_types: dao_council
                    .allowed_proposal_action_types
                    .unwrap_or_else(|| vec![ProposalActionType::UpgradeDao]),
                quorum: dao_council.quorum,
                threshold: dao_council.threshold,
            }))
        }
    }
}

// TODO: tests
pub fn validate_distribute_funds(msg: &DistributeFundsMsg) -> GovernanceControllerResult<()> {
    for asset in &msg.funds {
        match asset.info {
            AssetInfoBase::Native(_) | AssetInfoBase::Cw20(_) => {
                // no action, those assets are supported
            }
            AssetInfoBase::Cw1155(_, _) => {
                return Err(Std(StdError::generic_err(
                    "cw1155 is not supported at this time",
                )))
            }
            _ => return Err(Std(StdError::generic_err("unknown asset type"))),
        }
    }

    Ok(())
}

pub fn validate_no_duplicate_council_members(
    deps: Deps,
    members: Vec<String>,
) -> GovernanceControllerResult<Vec<Addr>> {
    // tracks whether we encountered a member or not
    let mut members_set: HashSet<Addr> = HashSet::new();

    // keeps members' validated addresses, in order in which we received them
    let mut member_addrs: Vec<Addr> = Vec::with_capacity(members.len());
    for member in members {
        let member_addr = deps.api.addr_validate(&member)?;
        if !members_set.insert(member_addr.clone()) {
            return Err(DuplicateCouncilMember { member });
        }
        member_addrs.push(member_addr);
    }

    Ok(member_addrs)
}

/// Check if allowed council proposal types contain dangerous types of actions that a council
/// shouldn't be allowed to do.
pub fn validate_allowed_council_proposal_types(
    proposal_action_types: Option<Vec<ProposalActionType>>,
) -> GovernanceControllerResult<()> {
    match proposal_action_types {
        None => Ok(()),
        Some(action_types) => {
            for action_type in action_types {
                match action_type {
                    ProposalActionType::UpdateGovConfig
                    | ProposalActionType::UpdateCouncil
                    | ProposalActionType::RequestFundingFromDao
                    | ProposalActionType::ExecuteMsgs
                    | ProposalActionType::ModifyMultisigMembership
                    | ProposalActionType::DistributeFunds
                    | ProposalActionType::UpdateMinimumWeightForRewards => {
                        return Err(UnsupportedCouncilProposalAction {
                            action: action_type,
                        });
                    }
                    ProposalActionType::UpdateMetadata
                    | ProposalActionType::UpdateAssetWhitelist
                    | ProposalActionType::UpdateNftWhitelist
                    | ProposalActionType::UpgradeDao => {
                        // allowed proposal action types
                    }
                }
            }
            Ok(())
        }
    }
}
