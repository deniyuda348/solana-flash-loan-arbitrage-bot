use anyhow::{Result, anyhow};
use anchor_client::solana_sdk::pubkey::Pubkey;
use std::{collections::HashMap, str::FromStr, sync::{Arc, Mutex}, time::Duration};
use solana_client::rpc_client::RpcClient;
use anchor_client::solana_sdk::commitment_config::CommitmentConfig;
use spl_token::solana_program::native_token::{lamports_to_sol, LAMPORTS_PER_SOL};
use tokio::time::{self, Instant};
use futures_util::{stream::StreamExt, SinkExt, Sink};
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
use yellowstone_grpc_proto::geyser::{
    subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest, SubscribeRequestPing,
    SubscribeRequestFilterTransactions, SubscribeUpdateTransaction, SubscribeUpdate,
};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use serde_json;
use maplit::hashmap;
use rand::random;
use bs58;

use crate::common::config::{
    JUPITER_PROGRAM,
    OKX_DEX_PROGRAM,
    AppState, 
    SwapConfig,
};
use crate::common::logger::Logger;
use crate::dex::dex_registry::DEXRegistry;
use crate::engine::pool_discovery::PoolCacheManager;
use colored::Colorize;

/// Configuration for arbitrage monitoring
pub struct ArbitrageConfig {
    /// Minimum price difference percentage to consider for arbitrage
    pub threshold_pct: f64,
    /// Minimum liquidity in lamports to consider for arbitrage
    pub min_liquidity: u64,
    /// Path to the pool cache file
    pub cache_path: String,
}

impl Default for ArbitrageConfig {
    fn default() -> Self {
        Self {
            threshold_pct: 1.5,
            min_liquidity: 10_000_000_000, // 10 SOL
            cache_path: "pool_cache.json".to_string(),
        }
    }
}

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

/// Heartbeat function to periodically send pings
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
            logger.log("[HEARTBEAT PING SENT]".blue().to_string());
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

/// Run the arbitrage monitor
pub async fn run_arbitrage_monitor(
    yellowstone_grpc_http: String,
    yellowstone_grpc_token: String,
    app_state: AppState,
    swap_config: SwapConfig,
    arbitrage_config: ArbitrageConfig,
) -> Result<(), String> {
    // Log the arbitrage configuration
    let logger = Logger::new("[ARBITRAGE-MONITOR] => ".blue().bold().to_string());

    logger.log(format!(
        "[CONFIG] => Threshold: {}%, Min Liquidity: {} SOL",
        arbitrage_config.threshold_pct,
        lamports_to_sol(arbitrage_config.min_liquidity)
    ).green().to_string());

    // Initialize RPC client for initial pool discovery
    let rpc_url = std::env::var("RPC_HTTP").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
    
    // Initialize pool cache manager
    let cache_path = &arbitrage_config.cache_path;
    let pool_cache_manager = match PoolCacheManager::new(cache_path) {
        Ok(manager) => Arc::new(manager),
        Err(e) => return Err(format!("Failed to initialize pool cache: {}", e)),
    };
    
    // Get list of token mints from the pool cache
    let token_mints = match pool_cache_manager.get_cache() {
        Ok(cache) => {
            let mints: Vec<Pubkey> = cache.get_all_token_mints().iter()
                .filter_map(|mint_str| Pubkey::from_str(mint_str).ok())
                .collect();
            
            if mints.is_empty() {
                return Err("No token mints found in pool cache. Please run preprocessing first.".to_string());
            }
            
            mints
        },
        Err(e) => return Err(format!("Failed to read pool cache: {}", e)),
    };
    
    // Log the tokens we're monitoring
    logger.log(format!(
        "[TOKEN MONITORING] => Tracking {} tokens for arbitrage opportunities",
        token_mints.len()
    ).green().to_string());
    
    for token_mint in &token_mints {
        logger.log(format!("\t * [TOKEN] => {}", token_mint).green().to_string());
    }
    
    // INITIAL SETTING FOR SUBSCRIBE
    // -----------------------------------------------------------------------------------------------------------------------------
    let mut client = GeyserGrpcClient::build_from_shared(yellowstone_grpc_http.clone())
        .map_err(|e| format!("Failed to build client: {}", e))?
        .x_token::<String>(Some(yellowstone_grpc_token.clone()))
        .map_err(|e| format!("Failed to set x_token: {}", e))?
        .tls_config(ClientTlsConfig::new().with_native_roots())
        .map_err(|e| format!("Failed to set tls config: {}", e))?
        .connect()
        .await
        .map_err(|e| format!("Failed to connect: {}", e))?;

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

    // Initialize DEX registry to get program IDs
    let dex_registry = DEXRegistry::new();
    
    // Prepare program IDs for monitoring - include all DEXes
    let mut program_ids = Vec::new();
    
    // Add all DEX program IDs to the monitoring list
    for dex in dex_registry.get_all_dexes() {
        program_ids.push(dex.program_id.to_string());
        logger.log(format!(
            "[MONITORING DEX] => {} ({})",
            dex.name, dex.program_id
        ).green().to_string());
    }

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
                    account_include: program_ids.clone(),
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

    // Use a HashMap to track token prices across different DEXes
    let token_prices = Arc::new(Mutex::new(HashMap::<String, HashMap<String, (f64, u64)>>::new()));

    logger.log("[STARTED. MONITORING FOR ARBITRAGE OPPORTUNITIES]...".blue().bold().to_string());

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

    // Start a background task to check for arbitrage opportunities periodically
    let token_prices_clone = Arc::clone(&token_prices);
    let logger_clone = logger.clone();
    let pool_cache_manager_clone = Arc::clone(&pool_cache_manager);
    let arbitrage_threshold = arbitrage_config.threshold_pct;
    let min_liquidity_value = arbitrage_config.min_liquidity;
    
    tokio::spawn(async move {
        let prices_clone = Arc::clone(&token_prices_clone);
        let arb_logger = logger_clone.clone();
        let cache_manager = Arc::clone(&pool_cache_manager_clone);
        
        // Create arbitrage checking interval - check every 5 seconds
        let mut interval = time::interval(Duration::from_secs(5));
        
        loop {
            interval.tick().await;
            
            // Check for arbitrage opportunities
            let opportunities = {
                let prices = prices_clone.lock().unwrap();
                let mut arb_opportunities = Vec::new();
                
                // Get the current cache
                let cache = match cache_manager.get_cache() {
                    Ok(c) => c,
                    Err(e) => {
                        arb_logger.log(format!("[CACHE ERROR] => {}", e).red().to_string());
                        continue;
                    }
                };
                
                for (token_mint, dex_prices) in prices.iter() {
                    // Need at least 2 DEXes to compare
                    if dex_prices.len() < 2 {
                        continue;
                    }
                    
                    // Convert to a vector for easier comparison
                    let dex_price_vec: Vec<(&String, &(f64, u64))> = dex_prices.iter().collect();
                    
                    for i in 0..dex_price_vec.len() {
                        for j in i+1..dex_price_vec.len() {
                            let (dex1, (price1, liquidity1)) = dex_price_vec[i];
                            let (dex2, (price2, liquidity2)) = dex_price_vec[j];
                            
                            // Calculate price difference percentage
                            let price_diff_pct = ((price1 - price2).abs() / price2) * 100.0;
                            
                            // Check if price difference exceeds threshold and both have sufficient liquidity
                            if price_diff_pct > arbitrage_threshold && 
                               *liquidity1 >= min_liquidity_value && 
                               *liquidity2 >= min_liquidity_value {
                                
                                // Determine buy and sell DEXes based on price
                                let (buy_dex, buy_price, sell_dex, sell_price) = if price1 < price2 {
                                    (dex1, price1, dex2, price2)
                                } else {
                                    (dex2, price2, dex1, price1)
                                };
                                
                                // Calculate expected profit percentage
                                let expected_profit_pct = ((sell_price - buy_price) / buy_price) * 100.0;
                                
                                // Find the pool IDs from the cache
                                let mut buy_pool_id = "unknown";
                                let mut sell_pool_id = "unknown";
                                
                                if let Some(pools) = cache.pools.get(token_mint) {
                                    for pool in pools {
                                        if pool.dex_name == *buy_dex {
                                            buy_pool_id = &pool.pool_id;
                                        } else if pool.dex_name == *sell_dex {
                                            sell_pool_id = &pool.pool_id;
                                        }
                                    }
                                }
                                
                                arb_opportunities.push((
                                    token_mint.clone(),
                                    buy_dex.clone(),
                                    *buy_price,
                                    buy_pool_id.to_string(),
                                    sell_dex.clone(),
                                    *sell_price,
                                    sell_pool_id.to_string(),
                                    expected_profit_pct
                                ));
                            }
                        }
                    }
                }
                
                arb_opportunities
            };
            
            // Log arbitrage opportunities
            if !opportunities.is_empty() {
                arb_logger.log(format!(
                    "[ARBITRAGE OPPORTUNITIES] => Found {} potential arbitrage trades",
                    opportunities.len()
                ).green().bold().to_string());
                
                for (token, buy_dex, buy_price, buy_pool, sell_dex, sell_price, sell_pool, profit) in opportunities {
                    arb_logger.log(format!(
                        "\n\t * [ARBITRAGE] => Token: {} \n\t * [BUY] => {} at ${:.6} (Pool: {}) \n\t * [SELL] => {} at ${:.6} (Pool: {}) \n\t * [PROFIT] => {:.2}%",
                        token, buy_dex, buy_price, buy_pool, sell_dex, sell_price, sell_pool, profit
                    ).cyan().to_string());
                    
                    // Here you would implement the actual arbitrage execution
                    // This would involve:
                    // 1. Buy the token on the cheaper DEX
                    // 2. Sell the token on the more expensive DEX
                    // 3. Calculate actual profit after fees
                    
                    // For now, just log that we would execute the trade
                    arb_logger.log(format!(
                        "\n\t * [WOULD EXECUTE] => Arbitrage trade for token {} between {} and {}",
                        token, buy_dex, sell_dex
                    ).yellow().to_string());
                    
                    // Save arbitrage opportunity to a file for later analysis
                    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
                    let record = serde_json::json!({
                        "timestamp": timestamp,
                        "token_mint": token,
                        "buy_dex": buy_dex,
                        "buy_price": buy_price,
                        "buy_pool": buy_pool,
                        "sell_dex": sell_dex,
                        "sell_price": sell_price,
                        "sell_pool": sell_pool,
                        "price_difference_pct": profit,
                        "min_liquidity": lamports_to_sol(min_liquidity_value)
                    });
                    
                    // Ensure the directory exists
                    let record_dir = "arbitrage_opportunities";
                    if !Path::new(record_dir).exists() {
                        if let Err(e) = fs::create_dir_all(record_dir) {
                            arb_logger.log(format!("[ERROR] => Failed to create directory: {}", e).red().to_string());
                        }
                    }
                    
                    // Write to file
                    let filename = format!("{}/arb_{}_{}.json", record_dir, token.split_at(8).0, timestamp);
                    if let Ok(mut file) = File::create(&filename) {
                        if let Err(e) = file.write_all(serde_json::to_string_pretty(&record).unwrap_or_default().as_bytes()) {
                            arb_logger.log(format!("[ERROR] => Failed to write to file: {}", e).red().to_string());
                        }
                    }
                }
            }
        }
    });

    // Add a connection health check task
    let logger_health = logger.clone(); 
    tokio::spawn(async move {
        let health_logger = logger_health.clone();
        let mut interval = time::interval(Duration::from_secs(300)); // 5 minutes
        
        loop {
            interval.tick().await;
            check_connection_health(&health_logger).await;
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
                        // Extract DEX program ID from transaction
                        if let Some(transaction) = txn.transaction.clone() {
                            if let Some(message) = transaction.transaction.and_then(|t| t.message) {
                                for instruction in message.instructions {
                                    let program_idx = instruction.program_id_index as usize;
                                    if let Some(program_id_bytes) = message.account_keys.get(program_idx) {
                                        if let Ok(program_id) = Pubkey::try_from(program_id_bytes.clone()) {
                                            // Check if this is a DEX program
                                            if let Some(dex) = dex_registry.find_dex_by_program_id(&program_id) {
                                                logger.log(format!(
                                                    "[TRANSACTION] => DEX: {}, Signature: {}",
                                                    dex.name,
                                                    bs58::encode(&transaction.signature).into_string()
                                                ).blue().to_string());
                                                
                                                // Extract pool information and token prices
                                                // This would involve parsing the transaction logs and data
                                                // For now, we'll just log that we detected a DEX transaction
                                                
                                                // In a real implementation, you would:
                                                // 1. Extract the token mint address
                                                // 2. Extract the pool information
                                                // 3. Calculate the token price based on the pool reserves
                                                // 4. Update the token_prices HashMap
                                                

                                                
                                                // Mock implementation for demonstration
                                                let mock_token_mint = "TokenMintAddress"; // TODO: Extract the token mint address
                                                let mock_price = 1.0 + (random::<f64>() * 0.1); // Random price between 1.0 and 1.1
                                                let mock_liquidity = 1_000_000_000; // 1 SOL
                                                
                                                // Update token prices
                                                {
                                                    let mut prices = token_prices.lock().unwrap();
                                                    let dex_prices = prices
                                                        .entry(mock_token_mint.to_string())
                                                        .or_insert_with(HashMap::new);
                                                    
                                                    dex_prices.insert(dex.name.clone(), (mock_price, mock_liquidity));
                                                }
                                                
                                                logger.log(format!(
                                                    "[PRICE UPDATE] => Token: {}, DEX: {}, Price: ${:.6}, Liquidity: {} SOL",
                                                    mock_token_mint, dex.name, mock_price, lamports_to_sol(mock_liquidity)
                                                ).green().to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }
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