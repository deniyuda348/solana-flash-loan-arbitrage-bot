use std::{str::FromStr, sync::Arc, time::Duration};
use anyhow::{anyhow, Result};
use colored::Colorize;
use std::cmp;
use std::env;
use crate::common::pool::get_program_acccounts_with_filter;
use anchor_client::solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    system_program,
};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{RpcFilterType, Memcmp, MemcmpEncodedBytes},
};
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account_idempotent,
};
use tokio::time::{Instant, sleep};

use crate::{
    common::{config::SwapConfig, logger::Logger},
    core::token,
};

const METEORA_DLMM_POOL_SIZE: u64 = 904;
const METEORA_DLMM_PROGRAM: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const METEORA_DLMM_TOKEN_MINT_X_POSITION: u64 = 88;
const METEORA_DLMM_TOKEN_MINT_Y_POSITION: u64 = 112;

#[derive(Debug)]
pub struct LiquidityPool {
    // Account Discriminator (8 bytes)
    pub parameters: StaticParameters,       // 32 bytes
    pub v_parameters: VariableParameters,   // 32 bytes
    pub bump_seed: u8,                     // 1 byte
    pub bin_step_seed: [u8; 2],            // 2 bytes
    pub pair_type: u8,                     // 1 byte
    pub active_id: i32,                    // 4 bytes
    pub bin_step: u16,                     // 2 bytes
    pub status: u8,                        // 1 byte
    pub require_base_factor_seed: u8,       // 1 byte
    pub base_factor_seed: [u8; 2],         // 2 bytes
    pub activation_type: u8,               // 1 byte
    pub creator_pool_on_off_control: u8,   // 1 byte
    pub token_x_mint: Pubkey,              // 32 bytes
    pub token_y_mint: Pubkey,              // 32 bytes
    pub reserve_x: Pubkey,                 // 32 bytes
    pub reserve_y: Pubkey,                 // 32 bytes
    pub protocol_fee: ProtocolFee,         // 16 bytes
    pub padding1: [u8; 32],                // 32 bytes
    pub reward_infos: [RewardInfo; 2],     // 2 * 112 = 224 bytes
    pub oracle: Pubkey,                    // 32 bytes
    pub bin_array_bitmap: [u64; 16],       // 128 bytes
    pub last_updated_at: i64,              // 8 bytes
    pub padding2: [u8; 32],                // 32 bytes
    pub pre_activation_swap_address: Pubkey, // 32 bytes
    pub base_key: Pubkey,                  // 32 bytes
    pub activation_point: u64,             // 8 bytes
    pub pre_activation_duration: u64,      // 8 bytes
    pub padding3: [u8; 8],                 // 8 bytes
    pub padding4: u64,                     // 8 bytes
    pub creator: Pubkey,                   // 32 bytes
    pub token_mint_x_program_flag: u8,     // 1 byte
    pub token_mint_y_program_flag: u8,     // 1 byte
    pub reserved: [u8; 22],                // 22 bytes
}

#[derive(Debug)]
pub struct StaticParameters {
    pub base_factor: u16,                  // 2 bytes
    pub filter_period: u16,                // 2 bytes
    pub decay_period: u16,                 // 2 bytes
    pub reduction_factor: u16,             // 2 bytes
    pub variable_fee_control: u32,         // 4 bytes
    pub max_volatility_accumulator: u32,   // 4 bytes
    pub min_bin_id: i32,                   // 4 bytes
    pub max_bin_id: i32,                   // 4 bytes
    pub protocol_share: u16,               // 2 bytes
    pub base_fee_power_factor: u8,         // 1 byte
    pub padding: [u8; 5],                  // 5 bytes
}

#[derive(Debug)]
pub struct VariableParameters {
    pub volatility_accumulator: u32,       // 4 bytes
    pub volatility_reference: u32,         // 4 bytes
    pub index_reference: i32,              // 4 bytes
    pub padding: [u8; 4],                  // 4 bytes
    pub last_update_timestamp: i64,        // 8 bytes
    pub padding1: [u8; 8],                 // 8 bytes
}

#[derive(Debug)]
pub struct ProtocolFee {
    pub amount_x: u64,                     // 8 bytes
    pub amount_y: u64,                     // 8 bytes
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RewardInfo {
    pub mint: Pubkey,                      // 32 bytes
    pub vault: Pubkey,                     // 32 bytes
    pub funder: Pubkey,                    // 32 bytes
    pub reward_duration: u64,              // 8 bytes
    pub reward_duration_end: u64,          // 8 bytes
    pub reward_rate: u64,                  // 8 bytes
    pub last_update_time: u64,             // 8 bytes
    pub cumulative_seconds_with_empty_liquidity_reward: u64, // 8 bytes
}

impl LiquidityPool {
    //reserve_x = 8+32*2+16=88
    //reserve_y=112
    
    pub fn get_pool_by_mint(mint: &str, wsol: &str) -> Result<Self> {
        let rpc_client = RpcClient::new(env::var("RPC_HTTP").unwrap());
        let mint_pubkey = Pubkey::from_str(mint)?;
        let wsol_address = Pubkey::from_str(wsol)?;
        let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM)?;
        
        let pools = rpc_client.get_program_accounts_with_config(
            &program_id,
            RpcProgramAccountsConfig {
                filters: Some(vec![
                    RpcFilterType::DataSize(METEORA_DLMM_POOL_SIZE),
                    solana_client::rpc_filter::RpcFilterType::Memcmp(Memcmp::new(
                        METEORA_DLMM_TOKEN_MINT_X_POSITION.try_into().unwrap(), 
                        MemcmpEncodedBytes::Base64(base64::encode(mint_pubkey.to_bytes()))
                    )),
                    solana_client::rpc_filter::RpcFilterType::Memcmp(Memcmp::new(
                        METEORA_DLMM_TOKEN_MINT_Y_POSITION.try_into().unwrap(), 
                        MemcmpEncodedBytes::Base64(base64::encode(wsol_address.to_bytes()))
                    )),
                ]),
                account_config: RpcAccountInfoConfig {
                    encoding: Some(UiAccountEncoding::Base64),
                    ..RpcAccountInfoConfig::default()
                },
                ..RpcProgramAccountsConfig::default()
            },
        )?;
        
        if pools.is_empty() {
            return Err(anyhow!("No Meteora DLMM pool found for the given mints"));
        }
        
        let (pubkey, account) = &pools[0];
        let pool_id = *pubkey;
        let data = &account.data;

        // Account discriminator (8 bytes)
        let _discriminator = &data[0..8];

        // StaticParameters (32 bytes)
        let parameters = StaticParameters {
            base_factor: u16::from_le_bytes(data[8..10].try_into()?),
            filter_period: u16::from_le_bytes(data[10..12].try_into()?),
            decay_period: u16::from_le_bytes(data[12..14].try_into()?),
            reduction_factor: u16::from_le_bytes(data[14..16].try_into()?),
            variable_fee_control: u32::from_le_bytes(data[16..20].try_into()?),
            max_volatility_accumulator: u32::from_le_bytes(data[20..24].try_into()?),
            min_bin_id: i32::from_le_bytes(data[24..28].try_into()?),
            max_bin_id: i32::from_le_bytes(data[28..32].try_into()?),
            protocol_share: u16::from_le_bytes(data[32..34].try_into()?),
            base_fee_power_factor: data[34],
            padding: [data[35], data[36], data[37], data[38], data[39]],
        };

        // VariableParameters (32 bytes)
        let v_parameters = VariableParameters {
            volatility_accumulator: u32::from_le_bytes(data[40..44].try_into()?),
            volatility_reference: u32::from_le_bytes(data[44..48].try_into()?),
            index_reference: i32::from_le_bytes(data[48..52].try_into()?),
            padding: [data[52], data[53], data[54], data[55]],
            last_update_timestamp: i64::from_le_bytes(data[56..64].try_into()?),
            padding1: [data[64], data[65], data[66], data[67], data[68], data[69], data[70], data[71]],
        };

        // Main fields
        let bump_seed = data[72];
        let bin_step_seed = [data[73], data[74]];
        let pair_type = data[75];
        let active_id = i32::from_le_bytes(data[76..80].try_into()?);
        let bin_step = u16::from_le_bytes(data[80..82].try_into()?);
        let status = data[82];
        let require_base_factor_seed = data[83];
        let base_factor_seed = [data[84], data[85]];
        let activation_type = data[86];
        let creator_pool_on_off_control = data[87];

        // Pubkey fields
        let token_x_mint = Pubkey::new_from_array(data[88..120].try_into()?);
        let token_y_mint = Pubkey::new_from_array(data[120..152].try_into()?);
        let reserve_x = Pubkey::new_from_array(data[152..184].try_into()?);
        let reserve_y = Pubkey::new_from_array(data[184..216].try_into()?);

        // ProtocolFee
        let protocol_fee = ProtocolFee {
            amount_x: u64::from_le_bytes(data[216..224].try_into()?),
            amount_y: u64::from_le_bytes(data[224..232].try_into()?),
        };

        // Padding and rewards
        let mut padding1 = [0u8; 32];
        padding1.copy_from_slice(&data[232..264]);

        let mut reward_infos = [RewardInfo::default(); 2];
        for i in 0..2 {
            let offset = 264 + i * 128;
            reward_infos[i] = RewardInfo {
                mint: Pubkey::new_from_array(data[offset..offset+32].try_into()?),
                vault: Pubkey::new_from_array(data[offset+32..offset+64].try_into()?),
                funder: Pubkey::new_from_array(data[offset+64..offset+96].try_into()?),
                reward_duration: u64::from_le_bytes(data[offset+96..offset+104].try_into()?),
                reward_duration_end: u64::from_le_bytes(data[offset+104..offset+112].try_into()?),
                reward_rate: u64::from_le_bytes(data[offset+112..offset+120].try_into()?),
                last_update_time: u64::from_le_bytes(data[offset+120..offset+128].try_into()?),
                cumulative_seconds_with_empty_liquidity_reward: u64::from_le_bytes(data[offset+128..offset+136].try_into()?),
            };
        }

        // Remaining fields
        let oracle = Pubkey::new_from_array(data[520..552].try_into()?);
        
        let mut bin_array_bitmap = [0u64; 16];
        for i in 0..16 {
            bin_array_bitmap[i] = u64::from_le_bytes(data[552+i*8..552+(i+1)*8].try_into()?);
        }

        let last_updated_at = i64::from_le_bytes(data[680..688].try_into()?);
        
        let mut padding2 = [0u8; 32];
        padding2.copy_from_slice(&data[688..720]);
        
        let pre_activation_swap_address = Pubkey::new_from_array(data[720..752].try_into()?);
        let base_key = Pubkey::new_from_array(data[752..784].try_into()?);
        let activation_point = u64::from_le_bytes(data[784..792].try_into()?);
        let pre_activation_duration = u64::from_le_bytes(data[792..800].try_into()?);
        
        let mut padding3 = [0u8; 8];
        padding3.copy_from_slice(&data[800..808]);
        
        let padding4 = u64::from_le_bytes(data[808..816].try_into()?);
        let creator = Pubkey::new_from_array(data[816..848].try_into()?);
        let token_mint_x_program_flag = data[848];
        let token_mint_y_program_flag = data[849];
        
        let mut reserved = [0u8; 22];
        reserved.copy_from_slice(&data[850..872]);

        Ok(Self {
            parameters,
            v_parameters,
            bump_seed,
            bin_step_seed,
            pair_type,
            active_id,
            bin_step,
            status,
            require_base_factor_seed,
            base_factor_seed,
            activation_type,
            creator_pool_on_off_control,
            token_x_mint,
            token_y_mint,
            reserve_x,
            reserve_y,
            protocol_fee,
            padding1,
            reward_infos,
            oracle,
            bin_array_bitmap,
            last_updated_at,
            padding2,
            pre_activation_swap_address,
            base_key,
            activation_point,
            pre_activation_duration,
            padding3,
            padding4,
            creator,
            token_mint_x_program_flag,
            token_mint_y_program_flag,
            reserved,
        })
    }
    
    pub fn get_token_price(&self) -> f64 {
        // Implement token price calculation logic here
        // This is a placeholder implementation
        0.0
    }
}