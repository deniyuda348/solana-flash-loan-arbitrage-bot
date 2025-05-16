use std::{str::FromStr, sync::Arc, time::Duration};
use anyhow::{anyhow, Result};
use colored::Colorize;
use std::cmp;
use std::env;
use anchor_client::solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    system_program,
    account::Account,
};
use solana_client::rpc_client::RpcClient;
use crate::common::pool::get_program_acccounts_with_filter;
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account_idempotent,
};
use tokio::time::{Instant, sleep};

use crate::{
    common::{config::SwapConfig, logger::Logger},
    core::token,
};
const WHIRLPOOLS_PROGRAM: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";
const WHIRLPOOLS_POOL_SIZE: u64 = 653;
const WHIRLPOOLS_TOKEN_MINT_A_POSITION: u64 = 101; 
const WHIRLPOOLS_TOKEN_MINT_B_POSITION: u64 = 181; 

#[derive(Debug)]
pub struct Whirlpool {
    // Account Discriminator (8 bytes) - not shown in JSON but present in account data
    pub whirlpools_config: Pubkey,        // 32 bytes
    pub whirlpool_bump: u8,               // 1 byte
    pub tick_spacing: u16,                // 2 bytes
    pub tick_spacing_seed: [u8; 2],       // 2 bytes
    pub fee_rate: u16,                    // 2 bytes
    pub protocol_fee_rate: u16,           // 2 bytes
    pub liquidity: u128,                  // 16 bytes
    pub sqrt_price: u128,                 // 16 bytes
    pub tick_current_index: i32,          // 4 bytes
    pub protocol_fee_owed_a: u64,         // 8 bytes
    pub protocol_fee_owed_b: u64,         // 8 bytes
    pub token_mint_a: Pubkey,             // 32 bytes
    pub token_vault_a: Pubkey,            // 32 bytes
    pub fee_growth_global_a: u128,        // 16 bytes
    pub token_mint_b: Pubkey,             // 32 bytes
    pub token_vault_b: Pubkey,            // 32 bytes
    pub fee_growth_global_b: u128,        // 16 bytes
    pub reward_last_updated_timestamp: u64, // 8 bytes
    pub reward_infos: [RewardInfo; 3],    // 3 * 104 = 312 bytes
}

#[derive(Debug, Default , Copy, Clone)]
pub struct RewardInfo {
    pub mint: Pubkey,                     // 32 bytes
    pub vault: Pubkey,                    // 32 bytes
    pub authority: Pubkey,                // 32 bytes
    pub emissions_per_second_x64: u128,   // 16 bytes
    pub growth_global_x64: u128,          // 16 bytes
}

impl Whirlpool{
    fn get_pool_by_mint(mint1: &str, mint2: &str) -> Result<Whirlpool> {
        let rpc_client = RpcClient::new(env::var("RPC_HTTP").unwrap());
        let mint1_pubkey = Pubkey::from_str(mint1)?;
        let mint2_pubkey = Pubkey::from_str(mint2)?;
        
        let pools = get_program_acccounts_with_filter(
            &rpc_client,
            &Pubkey::from_str(WHIRLPOOLS_PROGRAM)?,
            WHIRLPOOLS_POOL_SIZE,
            &WHIRLPOOLS_TOKEN_MINT_A_POSITION.try_into().unwrap(),
            &WHIRLPOOLS_TOKEN_MINT_B_POSITION.try_into().unwrap(),
            &mint1_pubkey,
            &mint2_pubkey
        )?;
        
        if pools.is_empty() {
            return Err(anyhow!("No Whirlpool found for the given mints"));
        }
        
        let (_, account) = &pools[0];
        let data = &account.data;

        // Account discriminator (8 bytes)
        let _discriminator = &data[0..8];
        // Fixed fields
        let whirlpools_config = Pubkey::new_from_array(data[8..40].try_into().unwrap());
        let whirlpool_bump = data[40];
        let tick_spacing = u16::from_le_bytes(data[41..43].try_into().unwrap());
        let tick_spacing_seed = [data[43], data[44]];
        let fee_rate = u16::from_le_bytes(data[45..47].try_into().unwrap());
        let protocol_fee_rate = u16::from_le_bytes(data[47..49].try_into().unwrap());
        let liquidity = u128::from_le_bytes(data[49..65].try_into().unwrap());
        let sqrt_price = u128::from_le_bytes(data[65..81].try_into().unwrap());
        let tick_current_index = i32::from_le_bytes(data[81..85].try_into().unwrap());
        let protocol_fee_owed_a = u64::from_le_bytes(data[85..93].try_into().unwrap());
        let protocol_fee_owed_b = u64::from_le_bytes(data[93..101].try_into().unwrap());
        let token_mint_a = Pubkey::new_from_array(data[101..133].try_into().unwrap());
        let token_vault_a = Pubkey::new_from_array(data[133..165].try_into().unwrap());
        let fee_growth_global_a = u128::from_le_bytes(data[165..181].try_into().unwrap());
        let token_mint_b = Pubkey::new_from_array(data[181..213].try_into().unwrap());
        let token_vault_b = Pubkey::new_from_array(data[213..245].try_into().unwrap());
        let fee_growth_global_b = u128::from_le_bytes(data[245..261].try_into().unwrap());
        let reward_last_updated_timestamp = u64::from_le_bytes(data[261..269].try_into().unwrap());

        // RewardInfos (3 items)
        let mut reward_infos = [RewardInfo::default(); 3];
        for i in 0..3 {
            let offset = 269 + i * 104;
            reward_infos[i] = RewardInfo {
                mint: Pubkey::new_from_array(data[offset..offset+32].try_into().unwrap()),
                vault: Pubkey::new_from_array(data[offset+32..offset+64].try_into().unwrap()),
                authority: Pubkey::new_from_array(data[offset+64..offset+96].try_into().unwrap()),
                emissions_per_second_x64: u128::from_le_bytes(data[offset+96..offset+112].try_into().unwrap()),
                growth_global_x64: u128::from_le_bytes(data[offset+112..offset+128].try_into().unwrap()),
            };
        }

        Ok(Whirlpool {
            whirlpools_config,
            whirlpool_bump,
            tick_spacing,
            tick_spacing_seed,
            fee_rate,
            protocol_fee_rate,
            liquidity,
            sqrt_price,
            tick_current_index,
            protocol_fee_owed_a,
            protocol_fee_owed_b,
            token_mint_a,
            token_vault_a,
            fee_growth_global_a,
            token_mint_b,
            token_vault_b,
            fee_growth_global_b,
            reward_last_updated_timestamp,
            reward_infos,
        })
    }
}

