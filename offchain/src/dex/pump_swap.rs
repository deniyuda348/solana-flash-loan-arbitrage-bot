use std::{str::FromStr, sync::Arc, time::Duration};
use anyhow::{anyhow, Result};
use colored::Colorize;
use std::cmp;
use std::env;
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{RpcFilterType, Memcmp},
};
use solana_client::rpc_filter::MemcmpEncodedBytes;

use anchor_client::solana_sdk::{
    pubkey::Pubkey,
};
use crate::common::pool::get_program_acccounts_with_filter;

// PumpSwap Constants
pub const PUMP_SWAP_PROGRAM: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
pub const PUMP_SWAP_POOL_SIZE: u64 = 300; // Approximate size, adjust if needed
pub const PUMP_SWAP_BASE_MINT_POSITION: u64 = 43; // Position of base mint in the account data
pub const PUMP_SWAP_QUOTE_MINT_POSITION: u64 = 75; // Position of quote mint in the account data
pub const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

/// A struct to represent the PumpSwap pool which uses constant product AMM
#[derive(Debug, Clone)]
pub struct PumpSwapPool {
    pub pool_id: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub lp_mint: Pubkey,
    pub pool_base_account: Pubkey,
    pub pool_quote_account: Pubkey,
    pub base_reserve: u64,
    pub quote_reserve: u64,
    pub coin_creator: Pubkey,
}

impl PumpSwapPool {
    /// Get the PumpSwap pool for a specific token mint
    pub fn get_pool_by_mint(mint1: &str, mint2: &str) -> Result<PumpSwapPool> {
        let rpc_client = RpcClient::new(env::var("RPC_HTTP").unwrap());
        let mint1_pubkey = Pubkey::from_str(mint1)?;
        let mint2_pubkey = Pubkey::from_str(mint2)?;
        
        // Try to find pool with mint1 as base and mint2 as quote
        let pools = get_program_acccounts_with_filter(
            &rpc_client,
            &Pubkey::from_str(PUMP_SWAP_PROGRAM)?,
            PUMP_SWAP_POOL_SIZE,
            &PUMP_SWAP_BASE_MINT_POSITION,
            &PUMP_SWAP_QUOTE_MINT_POSITION,
            &mint1_pubkey,
            &mint2_pubkey
        )?;
        
        // If no pools found, try with reversed order
        if pools.is_empty() {
            let pools = get_program_acccounts_with_filter(
                &rpc_client,
                &Pubkey::from_str(PUMP_SWAP_PROGRAM)?,
                PUMP_SWAP_POOL_SIZE,
                &PUMP_SWAP_BASE_MINT_POSITION,
                &PUMP_SWAP_QUOTE_MINT_POSITION,
                &mint2_pubkey,
                &mint1_pubkey
            )?;
            
            if pools.is_empty() {
                return Err(anyhow!("No PumpSwap pool found for the given mints"));
            }
        }
        
        // Take the first pool found
        let (pubkey, account) = &pools[0];
        let pool_id = *pubkey;
        let data = &account.data;
        
        // Parse the pool data according to the Pool struct layout
        let discriminator = &data[0..8]; // 8-byte discriminator
        let pool_bump = data[8]; // u8 at offset 8 (after 8-byte discriminator)
        let index = u16::from_le_bytes(data[9..11].try_into().unwrap()); // u16 at offset 9-10
        let creator = Pubkey::try_from(&data[11..43]).unwrap(); // Pubkey at offset 11-42 (32 bytes)
        let base_mint = Pubkey::try_from(&data[43..75]).unwrap(); // Pubkey at offset 43-74 (32 bytes)
        let quote_mint = Pubkey::try_from(&data[75..107]).unwrap(); // Pubkey at offset 75-106 (32 bytes)
        let lp_mint = Pubkey::try_from(&data[107..139]).unwrap(); // Pubkey at offset 107-138 (32 bytes)
        let pool_base_account = Pubkey::try_from(&data[139..171]).unwrap(); // Pubkey at offset 139-170 (32 bytes)
        let pool_quote_account = Pubkey::try_from(&data[171..203]).unwrap(); // Pubkey at offset 171-202 (32 bytes)
        let lp_supply = u64::from_le_bytes(data[203..211].try_into().unwrap()); // u64 at offset 203-210 (8 bytes)
        let coin_creator = Pubkey::try_from(&data[211..243]).unwrap(); // Pubkey at offset 211-242 (32 bytes)
        
        // Get token balances (reserves)
        let base_balance = match rpc_client.get_token_account_balance(&pool_base_account) {
            Ok(balance) => {
                match balance.ui_amount {
                    Some(amount) => (amount * (10f64.powf(balance.decimals as f64))) as u64,
                    None => 0,
                }
            },
            Err(_) => 0,
        };

        let quote_balance = match rpc_client.get_token_account_balance(&pool_quote_account) {
            Ok(balance) => {
                match balance.ui_amount {
                    Some(amount) => (amount * (10f64.powf(balance.decimals as f64))) as u64,
                    None => 0,
                }
            },
            Err(_) => 0,
        };
        
        Ok(PumpSwapPool {
            pool_id,
            base_mint,
            quote_mint,
            lp_mint,
            pool_base_account,
            pool_quote_account,
            base_reserve: base_balance,
            quote_reserve: quote_balance,
            coin_creator,
        })
    }
    
    /// Get the token price from the pool
    pub fn get_token_price(&self) -> f64 {
        if self.base_reserve == 0 {
            return 0.0;
        }
        
        self.quote_reserve as f64 / self.base_reserve as f64
    }
}