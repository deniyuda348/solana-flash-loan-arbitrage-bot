// File: src/lib.rs

use anchor_lang::prelude::*;
use crate::state::RaydiumSwapState;

pub mod ix_data;
pub mod state;
pub mod swaps;
pub mod error;

use swaps::*;
use swaps::me                                                                   use swaps::meteora::*;



declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[program]
pub mod tmp {
    use super::*;

    pub fn orca_swap(
        ctx: Context<OrcaSwap>,
        amount_in: u64,
        minimum_amount_out: u64,
    ) -> Result<()> {
        ctx.accounts.process_swap(amount_in, minimum_amount_out)
    }

    pub fn raydium_swap(
        ctx: Context<RaydiumSwap>,
        amount_in: u64,
        minimum_amount_out: u64,
    ) -> Result<()> {
        ctx.accounts.process_swap(amount_in, minimum_amount_out)
    }

    pub fn meteora_swap(
        ctx: Context<MeteoraSwap>,
        amount_in: u64,
        minimum_amount_out: u64,
    ) -> Result<()> {
        ctx.accounts.process_swap(amount_in, minimum_amount_out)
    }


}

