use std::{str::FromStr, sync::Arc};
use anyhow::{anyhow, Result};
use anchor_client::solana_sdk::pubkey::Pubkey;
use solana_client::rpc_client::RpcClient;
use std::env;

use crate::dex::raydium_amm::{RaydiumAMM as DexRaydiumAMM};
use crate::pools::Pool;

#[derive(Debug, Clone)]
pub struct RaydiumAMMPool {
    pub pool: DexRaydiumAMM,
    pub pool_id: Pubkey,
    pub token_base_balance: u64,
    pub token_quote_balance: u64,
}

impl RaydiumAMMPool {
    /// Create a new RaydiumAMMPool from the given mint addresses
    pub async fn new(mint_base: &str, mint_quote: &str) -> Result<Self> {
        let pool = DexRaydiumAMM::get_pool_by_mint(mint_base, mint_quote).await?;
        
        // Get token balances from the vault accounts
        let rpc_client = RpcClient::new(env::var("RPC_HTTP").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string()));
        
        let token_base_balance = match rpc_client.get_token_account_balance(&pool.base_vault) {
            Ok(balance) => {
                match balance.ui_amount {
                    Some(amount) => (amount * (10f64.powf(balance.decimals as f64))) as u64,
                    None => 0,
                }
            },
            Err(_) => 0,
        };

        let token_quote_balance = match rpc_client.get_token_account_balance(&pool.quote_vault) {
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
            token_base_balance,
            token_quote_balance,
        })
    }
}