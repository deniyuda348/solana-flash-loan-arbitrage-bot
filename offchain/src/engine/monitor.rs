use borsh::from_slice;
use maplit::hashmap;
use anchor_client::solana_sdk::signature::Signer;
use anchor_client::solana_sdk::{hash::Hash, pubkey::Pubkey, signature::Signature};
use spl_token::solana_program::native_token::{lamports_to_sol, LAMPORTS_PER_SOL};
use tokio::process::Command;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::{collections::HashSet, time::Duration};
use base64;
use once_cell::sync::Lazy;
use lazy_static::lazy_static;

use super::swap::{SwapDirection, SwapInType};
use crate::common::config::{
    JUPITER_PROGRAM,
    OKX_DEX_PROGRAM,
};
use crate::common::{    
    config::{AppState, LiquidityPool, Status, SwapConfig},
    logger::Logger,
};
use crate::core::tx;
use crate::dex::dex_registry::{DEXRegistry, identify_dex_from_pool};
use anyhow::{anyhow, Result};
use chrono::{Utc, Local};
use colored::Colorize;
use futures_util::stream::StreamExt;
use futures_util::{SinkExt, Sink};
use tokio::{
    sync::mpsc,
    task,
    time::{self, Instant},
};
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
// Import from crate::error instead
use crate::error::{ClientError, ClientResult};
use yellowstone_grpc_proto::geyser::{
    subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest, SubscribeRequestPing,
    SubscribeRequestFilterTransactions, SubscribeUpdateTransaction, SubscribeUpdate,
};
use std::str::FromStr;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use serde_json;
use crate::engine::swap::Pump;
use crate::engine::globals::{TOKEN_TRACKING, BUYING_ENABLED, MAX_WAIT_TIME, TokenTrackingInfo};
static DEX_LIST: Lazy<Vec<String>> = Lazy::new(|| vec![
    "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK".to_string(),
    "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C".to_string(),
    "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8".to_string(),
    "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc".to_string(),
    "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo".to_string(),
    "Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB".to_string(),
    "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA".to_string(),
]);
// static DEX_LIST: Lazy<Vec<Pubkey>> = Lazy::new(|| vec![
//     Pubkey::from_str("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK").expect("Failed to parse Pubkey"),
//     Pubkey::from_str("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C").expect("Failed to parse Pubkey"),
//     Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8").expect("Failed to parse Pubkey"),
//     Pubkey::from_str("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc").expect("Failed to parse Pubkey"),
//     Pubkey::from_str("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo").expect("Failed to parse Pubkey"),
//     Pubkey::from_str("Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB").expect("Failed to parse Pubkey"),
//     Pubkey::from_str("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA").expect("Failed to parse Pubkey"),
// ]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstructionType {
    SwapBuy,
    SwapSell,
    ArbitrageSwap
}

#[derive(Clone, Debug)]
pub struct BondingCurveInfo {
    pub bonding_curve: Pubkey,
    pub new_virtual_sol_reserve: u64,
    pub new_virtual_token_reserve: u64,
}

#[derive(Clone, Debug)]
pub struct PoolInfo {
    pub pool_id: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub pool_base_token_account: Pubkey,
    pub pool_quote_token_account: Pubkey,
    pub base_reserve: u64,
    pub quote_reserve: u64,
}

#[derive(Clone, Debug)]
pub struct TradeInfoFromToken {
    pub instruction_type: InstructionType,
    pub slot: u64,
    pub recent_blockhash: Hash,
    pub signature: String,
    pub target: String,
    pub mint: String,
    pub pool_info: Option<PoolInfo>, // Pool information for swap operations
    pub token_amount: f64,
    pub amount: Option<u64>,
    pub base_amount_in: Option<u64>, // For sell operations
    pub min_quote_amount_out: Option<u64>, // For sell operations
    pub base_amount_out: Option<u64>, // For buy operations
    pub max_quote_amount_in: Option<u64>, // For buy operations
    // New fields for arbitrage
    pub source_dex: Option<String>,
    pub target_dex: Option<String>,
    pub price_difference: Option<f64>,
    pub expected_profit: Option<f64>,
}

pub struct FilterConfig {
    program_ids: Vec<String>,
    dex_program_ids: Vec<String>,
    arbitrage_threshold_pct: f64,
    min_liquidity: u64,
}

#[derive(Clone, Debug)]
pub struct CopyTradeInfo {
    pub slot: u64,
    pub recent_blockhash: Hash,
    pub signature: String,
    pub target: String,
    pub mint: String,
    pub bonding_curve: String,
    pub volume_change: i64,
    pub bonding_curve_info: Option<BondingCurveInfo>,
}

impl TradeInfoFromToken {
    pub fn from_json(txn: SubscribeUpdateTransaction, log_messages: Vec<String>) -> Result<Self> {
        let slot = txn.slot;
        // Simplified implementation for now
        Ok(Self {
            instruction_type: InstructionType::ArbitrageSwap,
            slot,
            recent_blockhash: Hash::default(),
            signature: "signature".to_string(),
            target: "target".to_string(),
            mint: "mint".to_string(),
            pool_info: None,
            token_amount: 0.0,
            amount: None,
            base_amount_in: None,
            min_quote_amount_out: None,
            base_amount_out: None,
            max_quote_amount_in: None,
            source_dex: None,
            target_dex: None,
            price_difference: None,
            expected_profit: None,
        })
    }
}

/**
 * The following functions implement a ping-pong mechanism to keep the gRPC connection alive:
 * 
 * - process_stream_message: Handles incoming messages, responding to pings and logging pongs
 * - handle_ping_message: Sends a pong response when a ping is received
 * - send_heartbeat_ping: Proactively sends pings every 30 seconds
 * 
 * This ensures the connection stays active even during periods of inactivity,
 * preventing timeouts from the server or network infrastructure.
 */

/// Send a ping response when we receive a ping
async fn handle_ping_message(
    subscribe_tx: &Arc<tokio::sync::Mutex<impl Sink<SubscribeRequest, Error = impl std::fmt::Debug> + Unpin>>,
    logger: &Logger,
) -> Result<(), String> {
    let ping_request = SubscribeRequest {
        ping: Some(SubscribeRequestPing { id: 1 }),
        ..Default::default()
    };

    // Get a lock on the mutex
    let mut locked_tx = subscribe_tx.lock().await;
    
    // Send the ping response
    match locked_tx.send(ping_request).await {
        Ok(_) => {
            Ok(())
        },
        Err(e) => {
            Err(format!("Failed to send ping response: {:?}", e))
        }
    }
}

/// Process stream messages including ping-pong for keepalive
async fn process_stream_message(
    msg: &SubscribeUpdate,
    subscribe_tx: &Arc<tokio::sync::Mutex<impl Sink<SubscribeRequest, Error = impl std::fmt::Debug> + Unpin>>,
    logger: &Logger,
) -> Result<(), String> {
    match &msg.update_oneof {
        Some(UpdateOneof::Ping(_)) => {
            handle_ping_message(subscribe_tx, logger).await?;
        }
        Some(UpdateOneof::Pong(_)) => {
            // Just log that we received a pong
            logger.log("[PONG RECEIVED]".blue().to_string());
        }
        _ => {
            // Other message types, no special handling needed
        }
    }
    Ok(())
}

// Heartbeat function to periodically send pings
async fn send_heartbeat_ping(
    subscribe_tx: &Arc<tokio::sync::Mutex<impl Sink<SubscribeRequest, Error = impl std::fmt::Debug> + Unpin>>,
    logger: &Logger
) -> Result<(), String> {
    let ping_request = SubscribeRequest {
        ping: Some(SubscribeRequestPing { id: 1 }),
        ..Default::default()
    };
    
    // Get a lock on the mutex
    let mut locked_tx = subscribe_tx.lock().await;
    
    // Send the ping heartbeat
    match locked_tx.send(ping_request).await {
        Ok(_) => {
            Ok(())
        },
        Err(e) => {
            Err(format!("Failed to send heartbeat ping: {:?}", e))
        }
    }
}
/// Check connection health
async fn check_connection_health(logger: &Logger) {
    logger.log("[CONNECTION HEALTH] => Checking connection status...".blue().to_string());
    // In a real implementation, we would check if we've received any messages recently
    // For now, just log that we're checking
}

pub async fn arbitrage_monitor(
    yellowstone_grpc_http: String,
    yellowstone_grpc_token: String,
    app_state: AppState,
    swap_config: SwapConfig,
    time_exceed: u64,
    counter_limit: u64,
    min_dev_buy: u64,
    max_dev_buy: u64,
) -> Result<(), String> {
    // Log the copy trading configuration
    let logger = Logger::new("[Arbitrage-Trading] => ".blue().bold().to_string());

    let mut client = GeyserGrpcClient::build_from_shared(yellowstone_grpc_http.clone())
        .map_err(|e| format!("Failed to build client: {}", e))?
        .x_token::<String>(Some(yellowstone_grpc_token.clone()))
        .map_err(|e| format!("Failed to set x_token: {}", e))?
        .tls_config(ClientTlsConfig::new().with_native_roots())
        .map_err(|e| format!("Failed to set tls config: {}", e))?
        .connect()
        .await
        .map_err(|e| format!("Failed to connect: {}", e))?;

    // Create additional clones for later use in tasks
    let yellowstone_grpc_http = Arc::new(yellowstone_grpc_http);
    let yellowstone_grpc_token = Arc::new(yellowstone_grpc_token);
    let app_state = Arc::new(app_state);
    let swap_config = Arc::new(swap_config);

    let mut retry_count = 0;
    const MAX_RETRIES: u32 = 3;
    let (subscribe_tx, mut stream) = loop {
        match client.subscribe().await {
            Ok(pair) => break pair,
            Err(e) => {
                retry_count += 1;
                if retry_count >= MAX_RETRIES {
                    return Err(format!("Failed to subscribe after {} attempts: {}", MAX_RETRIES, e));
                }
                logger.log(format!(
                    "[CONNECTION ERROR] => Failed to subscribe (attempt {}/{}): {}. Retrying in 5 seconds...",
                    retry_count, MAX_RETRIES, e
                ).red().to_string());
                time::sleep(Duration::from_secs(5)).await;
            }
        }
    };

    // Convert to Arc to allow cloning across tasks
    let subscribe_tx = Arc::new(tokio::sync::Mutex::new(subscribe_tx));

    subscribe_tx
        .lock()
        .await
        .send(SubscribeRequest {
            slots: HashMap::new(),
            accounts: HashMap::new(),
            transactions: hashmap! {
                "All".to_owned() => SubscribeRequestFilterTransactions {
                    vote: None,
                    failed: Some(false),
                    signature: None,
                    account_include: DEX_LIST.clone(),
                    account_exclude: vec![JUPITER_PROGRAM.to_string(), OKX_DEX_PROGRAM.to_string()],
                    account_required: Vec::<String>::new()
                }
            },
            transactions_status: HashMap::new(),
            entry: HashMap::new(),
            blocks: HashMap::new(),
            blocks_meta: HashMap::new(),
            commitment: Some(CommitmentLevel::Processed as i32),
            accounts_data_slice: vec![],
            ping: None,
            from_slot: None,
        })
        .await
        .map_err(|e| format!("Failed to send subscribe request: {}", e))?;

    let existing_liquidity_pools = Arc::new(Mutex::new(HashSet::<LiquidityPool>::new()));

    let rpc_nonblocking_client = app_state.clone().rpc_nonblocking_client.clone();
    let rpc_client = app_state.clone().rpc_client.clone();
    let wallet = app_state.clone().wallet.clone();
    let swapx = Pump::new(
        rpc_nonblocking_client.clone(),
        rpc_client.clone(),
        wallet.clone(),
    );

    logger.log("[STARTED. MONITORING COPY TARGETS]...".blue().bold().to_string());
    
    // Set buying enabled to true at start
    {
        let mut buying_enabled = BUYING_ENABLED.lock().unwrap();
        *buying_enabled = true;
    }

    // After all setup and before the main loop, add a heartbeat ping task
    let subscribe_tx_clone = subscribe_tx.clone();
    let logger_clone = logger.clone();
    
    tokio::spawn(async move {
        let ping_logger = logger_clone.clone();
        let mut interval = time::interval(Duration::from_secs(30));
        
        loop {
            interval.tick().await;
            
            if let Err(e) = send_heartbeat_ping(&subscribe_tx_clone, &ping_logger).await {
                ping_logger.log(format!("[CONNECTION ERROR] => {}", e).red().to_string());
                break;
            }
        }
    });

    // Start a background task to check the status of tokens periodically
    let existing_liquidity_pools_clone = Arc::clone(&existing_liquidity_pools);
    let logger_clone = logger.clone();
    let app_state_for_background = Arc::clone(&app_state);
    let swap_config_for_background = Arc::clone(&swap_config);
    
    tokio::spawn(async move {
        let pools_clone = Arc::clone(&existing_liquidity_pools_clone);
        let check_logger = logger_clone.clone();
        let app_state_clone = Arc::clone(&app_state_for_background);
        let swap_config_clone = Arc::clone(&swap_config_for_background);
        
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            
            // Check if there are any bought tokens and if any have exceeded MAX_WAIT_TIME
            let now = Instant::now();
            let max_wait_time_millis = *MAX_WAIT_TIME.lock().unwrap();
            let max_wait_duration = Duration::from_millis(max_wait_time_millis);
            
            let (has_bought_tokens, tokens_to_sell) = {
                let pools = pools_clone.lock().unwrap();
                let bought_tokens: Vec<String> = pools.iter()
                    .filter(|pool| pool.status == Status::Bought)
                    .map(|pool| pool.mint.clone())
                    .collect();
                
                let timed_out_tokens: Vec<(String, Instant)> = pools.iter()
                    .filter(|pool| pool.status == Status::Bought && 
                           pool.timestamp.map_or(false, |ts| now.duration_since(ts) > max_wait_duration))
                    .map(|pool| (pool.mint.clone(), pool.timestamp.unwrap()))
                    .collect();
                
                // Log bought tokens that are waiting to be sold
                if !bought_tokens.is_empty() {
                    check_logger.log(format!(
                        "\n\t * [BUYING PAUSED] => Waiting for tokens to be sold: {:?}",
                        bought_tokens
                    ).yellow().to_string());
                }
                
                // Log tokens that have timed out and will be force-sold
                if !timed_out_tokens.is_empty() {
                    check_logger.log(format!(
                        "\n\t * [TIMEOUT DETECTED] => Will force-sell tokens that exceeded {} ms wait time: {:?}",
                        max_wait_time_millis,
                        timed_out_tokens.iter().map(|(mint, _)| mint).collect::<Vec<_>>()
                    ).red().bold().to_string());
                }
                
                (bought_tokens.len() > 0, timed_out_tokens)
            };
            
            // Update buying status
            {
                let mut buying_enabled = BUYING_ENABLED.lock().unwrap();
                *buying_enabled = !has_bought_tokens;
                
            }
            
            // Force-sell tokens that have exceeded MAX_WAIT_TIME
            for (mint, timestamp) in tokens_to_sell {
                // Clone the necessary state for this token
                let logger_for_selling = check_logger.clone();
                let pools_clone_for_selling = Arc::clone(&pools_clone);
                let app_state_for_selling = app_state_clone.clone();
                let swap_config_for_selling = swap_config_clone.clone();
                
                check_logger.log(format!(
                    "\n\t * [FORCE SELLING] => Token {} exceeded wait time (elapsed: {:?})",
                    mint, now.duration_since(timestamp)
                ).red().to_string());
                
                tokio::spawn(async move {
                    // Get the existing pool for this mint
                    let existing_pool = {
                        let pools = pools_clone_for_selling.lock().unwrap();
                        pools.iter()
                            .find(|pool| pool.mint == mint)
                            .cloned()
                            .unwrap_or(LiquidityPool {
                                mint: mint.clone(),
                                buy_price: 0_f64,
                                sell_price: 0_f64,
                                status: Status::Bought,
                                timestamp: Some(timestamp),
                            })
                    };
                    
                    // Set up sell config
                    let sell_config = SwapConfig {
                        swap_direction: SwapDirection::Sell,
                        in_type: SwapInType::Pct,
                        amount_in: 1_f64,  // Sell 100%
                        slippage: 100_u64, // Use full slippage
                        use_jito: swap_config_for_selling.clone().use_jito,
                    };
                    
                    // Create Pump instance for selling
                    let app_state_for_task = app_state_for_selling.clone();
                    let rpc_nonblocking_client = app_state_for_task.rpc_nonblocking_client.clone();
                    let rpc_client = app_state_for_task.rpc_client.clone();
                    let wallet = app_state_for_task.wallet.clone();
                    let swapx = Pump::new(rpc_nonblocking_client.clone(), rpc_client.clone(), wallet.clone());
                    
                    // Execute the sell operation
                    let start_time = Instant::now();
                    match swapx.build_swap_ixn_by_mint(&mint, None, sell_config, start_time).await {
                        Ok(result) => {
                            // Send instructions and confirm
                            let (keypair, instructions, token_price) = (result.0, result.1, result.2);
                            let recent_blockhash = match rpc_nonblocking_client.get_latest_blockhash().await {
                                Ok(hash) => hash,
                                Err(e) => {
                                    logger_for_selling.log(format!(
                                        "Error getting blockhash for force-selling {}: {}", mint, e
                                    ).red().to_string());
                                    return;
                                }
                            };
                            
                            match tx::new_signed_and_send_zeroslot(
                                recent_blockhash,
                                &keypair,
                                instructions,
                                &logger_for_selling,
                            ).await {
                                Ok(res) => {
                                    let sold_pool = LiquidityPool {
                                        mint: mint.clone(),
                                        buy_price: existing_pool.buy_price,
                                        sell_price: token_price,
                                        status: Status::Sold,
                                        timestamp: Some(Instant::now()),
                                    };
                                    
                                    // Update pool status to sold
                                    {
                                        let mut pools = pools_clone_for_selling.lock().unwrap();
                                        pools.retain(|pool| pool.mint != mint);
                                        pools.insert(sold_pool.clone());
                                    }
                                    
                                    logger_for_selling.log(format!(
                                        "\n\t * [SUCCESSFUL FORCE-SELL] => TX_HASH: (https://solscan.io/tx/{}) \n\t * [POOL] => ({}) \n\t * [SOLD] => {} :: ({:?}).",
                                        &res[0], mint, Utc::now(), start_time.elapsed()
                                    ).green().to_string());
                                    
                                    // Check if all tokens are sold
                                    let all_sold = {
                                        let pools = pools_clone_for_selling.lock().unwrap();
                                        !pools.iter().any(|pool| pool.status == Status::Bought)
                                    };
                                    
                                    if all_sold {
                                        // If all tokens are sold, enable buying
                                        let mut buying_enabled = BUYING_ENABLED.lock().unwrap();
                                        *buying_enabled = true;
                                        
                                        logger_for_selling.log(
                                            "\n\t * [BUYING ENABLED] => All tokens sold, can buy new tokens now"
                                            .green()
                                            .to_string(),
                                        );
                                    }
                                },
                                Err(e) => {
                                    logger_for_selling.log(format!(
                                        "Force-sell failed for {}: {}", mint, e
                                    ).red().to_string());
                                }
                            }
                        },
                        Err(e) => {
                            logger_for_selling.log(format!(
                                "Error building swap instruction for force-selling {}: {}", mint, e
                            ).red().to_string());
                        }
                    }
                });
            }
        }
    });

    // In copy_trader_pumpfun after the heartbeat task
    // Add a connection health check task
    let logger_health = logger.clone();
    tokio::spawn(async move {
        let health_logger = logger_health.clone();
        let mut interval = time::interval(Duration::from_secs(300)); // 5 minutes
        loop {
            interval.tick().await;
        }
    });

    // In copy_trader_pumpfun after the health check task:
    // Add a connection watchdog task
    let logger_watchdog = logger.clone();
    tokio::spawn(async move {
        let watchdog_logger = logger_watchdog;
        let mut interval = time::interval(Duration::from_secs(120)); // Check every 2 minutes
        
        loop {
            interval.tick().await;
            check_connection_health(&watchdog_logger).await;
        }
    });

    // In copy_trader_pumpfun after setting up the initial subscription and before the main event loop
    // Replace the PNL monitoring and auto-sell task with a pure price monitoring task
    let price_monitoring_pools_clone = Arc::clone(&existing_liquidity_pools);
    let price_monitoring_logger_clone = logger.clone();
    let price_monitoring_app_state_clone = Arc::clone(&app_state);
    let price_monitoring_token_tracking = Arc::clone(&TOKEN_TRACKING);

    tokio::spawn(async move {
        let pools_clone = Arc::clone(&price_monitoring_pools_clone);
        let monitor_logger = price_monitoring_logger_clone.clone();
        let app_state_clone = Arc::clone(&price_monitoring_app_state_clone);
        let token_tracking = Arc::clone(&price_monitoring_token_tracking);
        
        // Create price monitoring interval - check every 5 seconds
        let mut interval = time::interval(Duration::from_secs(5));
        
        loop {
            interval.tick().await;
            
            // Get current pools to check
            let tokens_to_check = {
                let pools = pools_clone.lock().unwrap();
                pools.iter()
                    .filter(|pool| pool.status == Status::Bought)
                    .map(|pool| pool.clone())
                    .collect::<Vec<LiquidityPool>>()
            };
            
            if tokens_to_check.is_empty() {
                continue;
            }
            
            monitor_logger.log(format!(
                "\n[PRICE MONITOR] => Checking prices for {} tokens",
                tokens_to_check.len()
            ).blue().to_string());
            
            // Check each token's current price
            for pool in tokens_to_check {
                let mint = pool.mint.clone();
                let buy_price = pool.buy_price;
                let bought_time = pool.timestamp.unwrap_or(Instant::now());
                let time_elapsed = Instant::now().duration_since(bought_time);
                
                // Clone necessary variables
                let logger_for_price = monitor_logger.clone();
                let token_tracking_clone = Arc::clone(&token_tracking);
                let app_state_for_price = app_state_clone.clone();
                
                // Create Pump instance for price checking
                let rpc_nonblocking_client = app_state_for_price.rpc_nonblocking_client.clone();
                let rpc_client = app_state_for_price.rpc_client.clone();
                let wallet = app_state_for_price.wallet.clone();
                let swapx = Pump::new(rpc_nonblocking_client.clone(), rpc_client.clone(), wallet.clone());
                
                // Execute as a separate task to avoid blocking price check loop
                tokio::spawn(async move {
                    // Get current price estimate
                    let current_price = match swapx.get_token_price(&mint).await {
                        Ok(price) => price,
                        Err(e) => {
                            logger_for_price.log(format!(
                                "[PRICE ERROR] => Failed to get current price for {}: {}",
                                mint, e
                            ).red().to_string());
                            return;
                        }
                    };
                    
                    // Calculate PNL for informational purposes
                    let pnl = if buy_price > 0.0 {
                        ((current_price - buy_price) / buy_price) * 100.0
                    } else {
                        0.0
                    };
                    
                    // Get or create token tracking info
                    let mut tracking_info = {
                        let mut tracking = token_tracking_clone.lock().unwrap();
                        tracking.entry(mint.clone()).or_insert_with(|| TokenTrackingInfo {
                            top_pnl: pnl,
                            last_price_check: Instant::now(),
                            price_history: Vec::new(),
                        }).clone()
                    };
                    
                    // Update top PNL if current PNL is higher (for informational purposes)
                    if pnl > tracking_info.top_pnl {
                        let mut tracking = token_tracking_clone.lock().unwrap();
                        if let Some(info) = tracking.get_mut(&mint) {
                            info.top_pnl = pnl;
                            // Add price to history
                            info.price_history.push((current_price, Instant::now()));
                            // Keep only the last 100 price points
                            if info.price_history.len() > 100 {
                                info.price_history.remove(0);
                            }
                        }
                        tracking_info.top_pnl = pnl;
                        
                        logger_for_price.log(format!(
                            "\n[PNL PEAK] => Token {} reached new peak PNL: {:.2}%",
                            mint, pnl
                        ).green().bold().to_string());
                    }
                    
                    // Log current price status
                    logger_for_price.log(format!(
                        "[PRICE STATUS] => Token: {} | Buy: ${:.6} | Current: ${:.6} | PNL: {:.2}% | Peak PNL: {:.2}% | Time: {:?}",
                        mint, buy_price, current_price, pnl, tracking_info.top_pnl, time_elapsed
                    ).cyan().to_string());
                    
                    // Update last price check time
                    {
                        let mut tracking = token_tracking_clone.lock().unwrap();
                        if let Some(info) = tracking.get_mut(&mint) {
                            info.last_price_check = Instant::now();
                            // Add price to history
                            info.price_history.push((current_price, Instant::now()));
                            // Keep only the last 100 price points
                            if info.price_history.len() > 100 {
                                info.price_history.remove(0);
                            }
                        }
                    }
                    
                    // Calculate price change rate over the last few data points
                    let price_change_rate = {
                        let tracking = token_tracking_clone.lock().unwrap();
                        if let Some(info) = tracking.get(&mint) {
                            if info.price_history.len() >= 2 {
                                let newest = &info.price_history[info.price_history.len() - 1];
                                let oldest = &info.price_history[0];
                                let time_diff = newest.1.duration_since(oldest.1).as_secs_f64();
                                if time_diff > 0.0 {
                                    (newest.0 - oldest.0) / time_diff
                                } else {
                                    0.0
                                }
                            } else {
                                0.0
                            }
                        } else {
                            0.0
                        }
                    };
                    
                    // Log price change rate
                    if price_change_rate != 0.0 {
                        logger_for_price.log(format!(
                            "[PRICE CHANGE RATE] => Token: {} | Rate: ${:.6}/sec",
                            mint, price_change_rate
                        ).yellow().to_string());
                    }
                });
            }
        }
    });

    while let Some(message) = stream.next().await {
        match message {
            Ok(msg) => {
                // Process ping/pong messages
                if let Err(e) = process_stream_message(&msg, &subscribe_tx, &logger).await {
                    logger.log(format!("Error handling stream message: {}", e).red().to_string());
                    continue;
                }
                
                // Process transaction messages
                if let Some(UpdateOneof::Transaction(txn)) = msg.update_oneof {
                    let start_time = Instant::now();
                    if let Some(log_messages) = txn
                        .clone()
                        .transaction
                        .and_then(|txn1| txn1.meta)
                        .map(|meta| meta.log_messages)
                    {
                        // Process transaction to extract trade information
                        let trade_info = match TradeInfoFromToken::from_json(txn.clone(), log_messages.clone()) {
                            Ok(info) => info,
                            Err(e) => {
                                logger.log(
                                    format!("Error in parsing txn: {}", e)
                                        .red()
                                        .italic()
                                        .to_string(),
                                );
                                continue;
                            }
                        };
                    }
                }
            }
            Err(error) => {
                logger.log(
                    format!("Yellowstone gRpc Error: {:?}", error)
                        .red()
                        .to_string(),
                );
                break;
            }
        }
    }
    Ok(())
}

