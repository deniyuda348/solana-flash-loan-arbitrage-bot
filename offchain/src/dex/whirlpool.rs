use std::{str::FromStr, sync::Arc};
use anyhow::{anyhow, Result};
use anchor_client::solana_sdk::pubkey::Pubkey;
use solana_client::rpc_client::RpcClient;
use std::env;

use crate::dex::whirlpool::{Whirlpool as DexWhirlpool};
use crate::pools::Pool;

#[derive(Debug, Clone)]
pub struct WhirlpoolPool {
    pub pool: DexWhirlpool,
    pub pool_id: Pubkey,
    pub token_a_balance: u64,
    pub token_b_balance: u64,
}

impl WhirlpoolPool {
    /// Create a new WhirlpoolPool from the given mint addresses
    pub fn new(mint_a: &str, mint_b: &str) -> Result<Self> {
        let pool = DexWhirlpool::get_pool_by_mint(mint_a, mint_b)?;
        
        // Get token balances from the vault accounts
        let rpc_client = RpcClient::new(env::var("RPC_HTTP").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string()));
        
        let token_a_balance = match rpc_client.get_token_account_balance(&pool.token_vault_a) {
            Ok(balance) => {
                match balance.ui_amount {
                    Some(amount) => (amount * (10f64.powf(balance.decimals as f64))) as u64,
                    None => 0,
                }
            },
            Err(_) => 0,
        };

        let token_b_balance = match rpc_client.get_token_account_balance(&pool.token_vault_b) {
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
            token_a_balance,
            token_b_balance,
        })
    }
}

impl Pool for WhirlpoolPool {
    fn get_id(&self) -> Pubkey {
        self.pool_id
    }
    
    fn get_dex_name(&self) -> &str {
        "whirlpool"
    }
    
    fn get_token_a_mint(&self) -> Pubkey {
        self.pool.token_mint_a
    }
    
    fn get_token_b_mint(&self) -> Pubkey {
        self.pool.token_mint_b
    }
    
    fn get_token_a_reserve(&self) -> Pubkey {
        self.pool.token_vault_a
    }
    
    fn get_token_b_reserve(&self) -> Pubkey {
        self.pool.token_vault_b
    }
    
    fn get_token_price(&self) -> f64 {
        // For Whirlpool, we can calculate the price from the sqrt_price
        // sqrt_price is stored as a Q64.64 fixed-point number
        let sqrt_price_x64 = self.pool.sqrt_price;
        
        // Convert to floating point
        let sqrt_price = (sqrt_price_x64 as f64) / (1u128 << 64) as f64;
        
        // Square to get the price
        sqrt_price * sqrt_price
    }
    
    fn get_estimated_output_amount(&self, input_amount: u64, is_a_to_b: bool) -> Result<u64> {
        // Whirlpool uses concentrated liquidity which makes exact output calculation complex
        // This is a simplified approach using the current price and liquidity
        
        if input_amount == 0 {
            return Ok(0);
        }
        
        // Get the current price
        let price = self.get_token_price();
        
        if price == 0.0 {
            return Err(anyhow!("Zero price in pool"));
        }
        
        // Apply the fee
        let fee_bps = self.pool.fee_rate;
        let input_amount_with_fee = (input_amount as f64) * (1.0 - (fee_bps as f64 / 10000.0));
        
        // Calculate output based on price
        let output_amount = if is_a_to_b {
            input_amount_with_fee * price
        } else {
            input_amount_with_fee / price
        };
        
        Ok(output_amount as u64)
    }
    
    fn get_fee_rate(&self) -> u16 {
        self.pool.fee_rate
    }
    
    fn is_active(&self) -> bool {
        self.token_a_balance > 0 && self.token_b_balance > 0 && self.pool.liquidity > 0
    }
    
    fn get_pool_type(&self) -> &str {
        "concentrated_liquidity"
    }
} 