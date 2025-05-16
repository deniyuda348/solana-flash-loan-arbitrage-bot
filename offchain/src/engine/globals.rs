use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use lazy_static::lazy_static;
use tokio::time::Instant;

#[derive(Clone, Debug)]
pub struct TokenTrackingInfo {
    pub top_pnl: f64,
    pub last_price_check: Instant,
    pub price_history: Vec<(f64, Instant)>,  // Store price history with timestamps
}

// Add global variables for token tracking and buying status
lazy_static! {
    pub static ref TOKEN_TRACKING: Arc<Mutex<HashMap<String, TokenTrackingInfo>>> = Arc::new(Mutex::new(HashMap::new()));
    pub static ref BUYING_ENABLED: Mutex<bool> = Mutex::new(true);
    pub static ref MAX_WAIT_TIME: Mutex<u64> = Mutex::new(300000); // 5 minutes in milliseconds
}

// Constants
pub const LOG_INSTRUCTION: bool = true; 