use solana_vntr_sniper::{
    common::{config::Config, constants::RUN_MSG},
    dex::dex_registry::DEXRegistry,
    engine::arbitrage_monitor::{run_arbitrage_monitor, ArbitrageConfig},
    engine::preprocess::{run_preprocessing, PreprocessConfig},
};
use solana_vntr_sniper::common::config::SwapConfig;
use solana_vntr_sniper::engine::swap::SwapDirection;
use solana_vntr_sniper::engine::swap::SwapInType;
use solana_client::rpc_client::RpcClient;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use clap::{Parser, Subcommand};
use anyhow::{Result, anyhow};

#[derive(Parser)]
#[command(name = "arbitrage-bot")]
#[command(about = "Solana arbitrage bot for cross-DEX trading")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run preprocessing to fetch trending tokens and discover pools
    Preprocess {
        /// Number of trending tokens to fetch
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
        
        /// Path to save the pool cache
        #[arg(short, long, default_value = "pool_cache.json")]
        cache_path: String,
    },
    /// Run the arbitrage monitor
    Monitor {
        /// Minimum price difference percentage to consider for arbitrage
        #[arg(short, long, default_value_t = 1.5)]
        threshold: f64,
        
        /// Minimum liquidity in SOL to consider for arbitrage
        #[arg(short, long, default_value_t = 10.0)]
        min_liquidity: f64,
        
        /// Path to the pool cache file
        #[arg(short, long, default_value = "pool_cache.json")]
        cache_path: String,
    },
    /// Load and save pool data for different DEXes
    Pools {
        /// Save to specific DEX directories
        #[arg(short, long, default_value_t = true)]
        save: bool,
    },
}

/// Save pools information to respective directories
async fn save_pool_data() -> Result<()> {
    println!("ARBITRAGE BOT: Saving pool data for different DEXes");
    
    // Create RPC client
    let rpc_url = env::var("RPC_HTTP").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let rpc_client = RpcClient::new(&rpc_url);
    
    // DEX directories to save pool data
    let dex_dirs = [
        "pools/meteora_dlmm",
        "pools/meteora_pools",
        "pools/orca",
        "pools/pump_swap",
        "pools/raydium_amm",
        "pools/raydium_clmm",
        "pools/raydium_cpmm",
    ];
    
    // Ensure directories exist
    for dir in &dex_dirs {
        if !Path::new(dir).exists() {
            fs::create_dir_all(dir)?;
            println!("Created directory: {}", dir);
        }
    }
    
    // Get DEX registry
    let dex_registry = DEXRegistry::new();
    
    // Process each DEX
    for dex in dex_registry.get_all_dexes() {
        let dex_name = dex.name.as_str();
        
        // Determine the target directory for this DEX
        let target_dir = match dex_name {
            "meteora_dlmm" => Some("pools/meteora_dlmm"),
            "meteora_pools" => Some("pools/meteora_pools"),
            "orca" => Some("pools/orca"),
            "pumpswap" => Some("pools/pump_swap"),
            "raydium_amm" => Some("pools/raydium_amm"),
            "raydium_clmm" => Some("pools/raydium_clmm"),
            "raydium_cpmm" => Some("pools/raydium_cpmm"),
            _ => None,
        };
        
        // Skip DEXes without a target directory
        if target_dir.is_none() {
            continue;
        }
        
        let target_dir = target_dir.unwrap();
        println!("Processing {}...", dex_name);
        
        // Try to fetch pool accounts for this DEX
        // This fetches all accounts owned by the DEX program
        match rpc_client.get_program_accounts(&dex.program_id) {
            Ok(accounts) => {
                println!("Found {} accounts for {}", accounts.len(), dex_name);
                
                // Process each account - we'll only save a minimal template for now
                for (pubkey, _account) in accounts.iter().take(20) { // Limit to prevent overload
                    let pool_id = pubkey.to_string();
                    let file_name = format!("{}_{}.json", pool_id, dex_name);
                    let file_path = format!("{}/{}", target_dir, file_name);
                    
                    // Create an empty JSON file for now - you can expand this
                    // to include parsed pool data in the future
                    let pool_data = serde_json::json!({
                        "pool_id": pool_id,
                        "dex": dex_name,
                        "program_id": dex.program_id.to_string()
                    });
                    
                    // Save the file
                    let mut file = File::create(&file_path)?;
                    file.write_all(serde_json::to_string_pretty(&pool_data)?.as_bytes())?;
                    
                    println!("Saved pool data: {}", file_path);
                }
            },
            Err(e) => {
                println!("Error fetching accounts for {}: {}", dex_name, e);
            }
        }
    }
    
    println!("Pool data saved successfully!");
    Ok(())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    
    /* Initial Settings */
    let config = Config::new().await;
    let config = config.lock().await;

    match cli.command {
        Commands::Preprocess { limit, cache_path } => {
            println!("ARBITRAGE BOT: Preprocessing - Fetching trending tokens and discovering pools");
            
            // Create RPC client for preprocessing
            let rpc_url = env::var("RPC_HTTP").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
            let rpc_client = RpcClient::new(&rpc_url);
            
            // Create preprocessing config
            let preprocess_config = PreprocessConfig {
                trending_token_limit: limit,
                cache_path,
                ..PreprocessConfig::default()
            };
            
            // Run preprocessing
            match run_preprocessing(&rpc_client, &preprocess_config).await {
                Ok(_) => println!("Preprocessing completed successfully"),
                Err(e) => eprintln!("Preprocessing error: {}", e),
            }
        },
        Commands::Monitor { threshold, min_liquidity, cache_path } => {
            println!("ARBITRAGE BOT: Monitoring token prices across multiple DEXes");
            
            /* Display supported DEXes */
            let dex_registry = DEXRegistry::new();
            println!("Tracking DEXes:");
            for dex in dex_registry.get_all_dexes() {
                println!("  - {} ({})", dex.name, dex.program_id);
            }
            
            /* Setup swap config for arbitrage */
            let swap_config = SwapConfig {
                swap_direction: SwapDirection::Buy,
                in_type: SwapInType::Qty,
                amount_in: 0.1, // Default to 0.1 SOL per trade
                slippage: 50, // 0.5% slippage
                use_jito: false, // Don't use Jito MEV protection by default
            };
            
            // Convert min_liquidity from SOL to lamports
            let min_liquidity_lamports = (min_liquidity * 1_000_000_000.0) as u64;
            
            // Create arbitrage config
            let arbitrage_config = ArbitrageConfig {
                threshold_pct: threshold,
                min_liquidity: min_liquidity_lamports,
                cache_path,
            };
            
            match run_arbitrage_monitor(
                config.yellowstone_grpc_http.clone(),
                config.yellowstone_grpc_token.clone(),
                config.app_state.clone(),
                swap_config,
                arbitrage_config,
            ).await {
                Ok(_) => println!("Arbitrage monitor completed successfully"),
                Err(e) => eprintln!("Arbitrage monitor error: {}", e),
            }
        },
        Commands::Pools { save } => {
            if save {
                match save_pool_data().await {
                    Ok(_) => println!("Pool data saved successfully"),
                    Err(e) => eprintln!("Error saving pool data: {}", e),
                }
            } else {
                println!("Pool data loading is pending implementation");
            }
        }
    }
}
