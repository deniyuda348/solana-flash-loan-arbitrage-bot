use std::{str::FromStr, sync::Arc};
use anyhow::{anyhow, Result};
use anchor_client::solana_sdk::pubkey::Pubkey;
use solana_client::rpc_client::RpcClient;
use std::env;

use crate::dex::meteora_pools::{MeteoraPool as DexMeteoraPool, METEORA_POOLS_PROGRAM};
use crate::pools::Pool;

#[derive(Debug, Clone)]
pub struct MeteoraPool {
    pub pool: DexMeteoraPool,
    pub pool_id: Pubkey,
    pub token_a_balance: u64,
    pub token_b_balance: u64,
}

impl MeteoraPool {
    /// Create a new MeteoraPool from the given mint addresses
    pub async fn new(mint_a: &str, mint_b: &str) -> Result<Self> {
        let pool = DexMeteoraPool::get_pool_by_mint(mint_a, mint_b).await?;
        
        // Get token balances from the vault accounts
        let rpc_client = RpcClient::new(env::var("RPC_HTTP").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string()));
        
        let token_a_balance = match rpc_client.get_token_account_balance(&pool.a_vault) {
            Ok(balance) => {
                match balance.ui_amount {
                    Some(amount) => (amount * (10f64.powf(balance.decimals as f64))) as u64,
                    None => 0,
                }
            },
            Err(_) => 0,
        };

        let token_b_balance = match rpc_client.get_token_account_balance(&pool.b_vault) {
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

impl Pool for MeteoraPool {
    fn get_id(&self) -> Pubkey {
        self.pool_id
    }
    
    fn get_dex_name(&self) -> &str {
        "meteora_pools"
    }
    
    fn get_token_a_mint(&self) -> Pubkey {
        self.pool.token_a_mint
    }
    
    fn get_token_b_mint(&self) -> Pubkey {
        self.pool.token_b_mint
    }
    
    fn get_token_a_reserve(&self) -> Pubkey {
        self.pool.a_vault
    }
    
    fn get_token_b_reserve(&self) -> Pubkey {
        self.pool.b_vault
    }
    
    fn get_token_price(&self) -> f64 {
        if self.token_a_balance == 0 {
            return 0.0;
        }
        
        self.token_b_balance as f64 / self.token_a_balance as f64
    }
    
    fn get_estimated_output_amount(&self, input_amount: u64, is_a_to_b: bool) -> Result<u64> {
        if input_amount == 0 {
            return Ok(0);
        }
        
        let (reserve_in, reserve_out) = if is_a_to_b {
            (self.token_a_balance, self.token_b_balance)
        } else {
            (self.token_b_balance, self.token_a_balance)
        };
        
        if reserve_in == 0 || reserve_out == 0 {
            return Err(anyhow!("Zero reserves in pool"));
        }
        
        // Get fee from the pool
        let fee_numerator = self.pool.fees.trade_fee_numerator;
        let fee_denominator = self.pool.fees.trade_fee_denominator;
        
        // Calculate fee in basis points
        let fee_bps = if fee_denominator == 0 { 
            30 // Default to 0.3% if not set
        } else {
            ((fee_numerator as f64 / fee_denominator as f64) * 10000.0) as u16
        };
        
        // Apply fee
        let input_amount_with_fee = (input_amount as u128) * (10000 - fee_bps as u128) / 10000;
        
        // Meteora pools can be constant product or stable curve
        // For simplicity, we'll use constant product formula for both
        let numerator = input_amount_with_fee * (reserve_out as u128);
        let denominator = (reserve_in as u128) + input_amount_with_fee;
        
        let output_amount = numerator / denominator;
        
        Ok(output_amount as u64)
    }
    
    fn get_fee_rate(&self) -> u16 {
        // Calculate fee in basis points
        let fee_numerator = self.pool.fees.trade_fee_numerator;
        let fee_denominator = self.pool.fees.trade_fee_denominator;
        
        if fee_denominator == 0 { 
            30 // Default to 0.3% if not set
        } else {
            ((fee_numerator as f64 / fee_denominator as f64) * 10000.0) as u16
        }
    }
    
    fn is_active(&self) -> bool {
        self.token_a_balance > 0 && self.token_b_balance > 0 && self.pool.enabled
    }
    
    fn get_pool_type(&self) -> &str {
        match self.pool.curve_type {
            crate::dex::meteora_pools::CurveType::ConstantProduct => "constant_product",
            _ => "stable_curve",
        }
    }
} 