use std::{str::FromStr, sync::Arc};
use anyhow::{anyhow, Result};
use anchor_client::solana_sdk::pubkey::Pubkey;
use solana_client::rpc_client::RpcClient;
use std::env;

use crate::dex::meteora_dlmm::{LiquidityPool as DexMeteoraDLMMPool};
use crate::pools::Pool;

#[derive(Debug, Clone)]
pub struct MeteoraDLMMPool {
    pub pool: DexMeteoraDLMMPool,
    pub pool_id: Pubkey,
    pub token_x_balance: u64,
    pub token_y_balance: u64,
}

impl MeteoraDLMMPool {
    /// Create a new MeteoraDLMMPool from the given mint addresses
    pub fn new(mint_x: &str, mint_y: &str) -> Result<Self> {
        let pool = DexMeteoraDLMMPool::get_pool_by_mint(mint_x, mint_y)?;
        
        // Get token balances from the reserve accounts
        let rpc_client = RpcClient::new(env::var("RPC_HTTP").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string()));
        
        let token_x_balance = match rpc_client.get_token_account_balance(&pool.reserve_x) {
            Ok(balance) => {
                match balance.ui_amount {
                    Some(amount) => (amount * (10f64.powf(balance.decimals as f64))) as u64,
                    None => 0,
                }
            },
            Err(_) => 0,
        };

        let token_y_balance = match rpc_client.get_token_account_balance(&pool.reserve_y) {
            Ok(balance) => {
                match balance.ui_amount {
                    Some(amount) => (amount * (10f64.powf(balance.decimals as f64))) as u64,
                    None => 0,
                }
            },
            Err(_) => 0,
        };
        
        Ok(Self {
            pool_id: Pubkey::default(), // We don't have the pool ID in the original structure
            pool,
            token_x_balance,
            token_y_balance,
        })
    }
}