use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use reqwest::Client;
use anchor_client::solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// Structure for token data from Jupiter API
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JupiterToken {
    pub address: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    #[serde(rename = "logoURI")]
    pub logo_uri: Option<String>,
    pub tags: Vec<String>,
    #[serde(rename = "daily_volume")]
    pub daily_volume: Option<f64>,
}

/// Structure for token metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMetadata {
    pub mint: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub logo_uri: Option<String>,
    pub daily_volume: Option<f64>,
}

/// Fetch trending tokens from Jupiter API
pub async fn fetch_trending_tokens(limit: usize) -> Result<Vec<TokenMetadata>> {
    let client = Client::new();
    let url = "https://tokens.jup.ag/tokens?tags=birdeye-trending";
    
    println!("Fetching trending tokens from Jupiter API...");
    
    let response = client.get(url)
        .send()
        .await
        .map_err(|e| anyhow!("Failed to fetch trending tokens: {}", e))?;
    
    if !response.status().is_success() {
        return Err(anyhow!(
            "Failed to fetch trending tokens: HTTP {}",
            response.status()
        ));
    }
    
    let tokens: Vec<JupiterToken> = response
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse trending tokens response: {}", e))?;
    
    // Convert to our internal format and validate pubkeys
    let mut valid_tokens = Vec::new();
    
    for token in tokens {
        // Validate that the address is a valid Solana pubkey
        if let Ok(_pubkey) = Pubkey::from_str(&token.address) {
            valid_tokens.push(TokenMetadata {
                mint: token.address,
                name: token.name,
                symbol: token.symbol,
                decimals: token.decimals,
                logo_uri: token.logo_uri,
                daily_volume: token.daily_volume,
            });
        }
    }
    
    // Sort by daily volume (descending) and take the top N
    valid_tokens.sort_by(|a, b| {
        let vol_a = a.daily_volume.unwrap_or(0.0);
        let vol_b = b.daily_volume.unwrap_or(0.0);
        vol_b.partial_cmp(&vol_a).unwrap_or(std::cmp::Ordering::Equal)
    });
    
    if limit > 0 && valid_tokens.len() > limit {
        valid_tokens.truncate(limit);
    }
    
    println!("Found {} valid trending tokens", valid_tokens.len());
    
    Ok(valid_tokens)
}

/// Get token mint addresses from token metadata
pub fn get_token_mints(tokens: &[TokenMetadata]) -> Vec<Pubkey> {
    tokens
        .iter()
        .filter_map(|token| Pubkey::from_str(&token.mint).ok())
        .collect()
} 