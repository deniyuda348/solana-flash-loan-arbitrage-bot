use anchor_lang::prelude::*;
use anchor_lang::solana_program::{instruction::Instruction, program::invoke};
use anchor_spl::token::{Token, TokenAccount};

use crate::error::ErrorCode;
use crate::state::RaydiumSwapState;

// Raydium AMM program ID
pub const RAYDIUM_AMM_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";

#[derive(Accounts)]
pub struct RaydiumClmm<'info> {
    pub bump: [u8; 1],
    pub amm_config: Pubkey,
    pub owner: Pubkey,

    pub token_mint_0: Pubkey,
    pub token_mint_1: Pubkey,

    pub token_vault_0: Pubkey,
    pub token_vault_1: Pubkey,

    pub observation_key: Pubkey,

    pub mint_decimals_0: u8,
    pub mint_decimals_1: u8,

    pub tick_spacing: u16,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current: i32,

    pub padding3: u16,
    pub padding4: u16,

    pub fee_growth_global_0_x64: u128,
    pub fee_growth_global_1_x64: u128,

    pub protocol_fees_token_0: u64,
    pub protocol_fees_token_1: u64,

    pub swap_in_amount_token_0: u128,
    pub swap_out_amount_token_1: u128,
    pub swap_in_amount_token_1: u128,
    pub swap_out_amount_token_0: u128,

    pub status: u8,
    pub padding: [u8; 7],

    pub reward_infos: [RewardInfo; REWARD_NUM],

    pub tick_array_bitmap: [u64; 16],

    pub total_fees_token_0: u64,
    pub total_fees_claimed_token_0: u64,
    pub total_fees_token_1: u64,
    pub total_fees_claimed_token_1: u64,

    pub fund_fees_token_0: u64,
    pub fund_fees_token_1: u64,

    pub open_time: u64,
    pub recent_epoch: u64,

    pub padding1: [u64; 24],
    pub padding2: [u64; 32],
}

impl RaydiumClmm {
    pub fn load_checked(data: &[u8]) -> Result<Self> {
        if data.len() < 8 + 32 + 32 + 32 + 32 + 32 + 32 + 4 + 2 {
            return Err(anyhow::anyhow!(
                "Invalid data length for RaydiumClmmPoolState"
            ));
        }

        let data = &data[8..]; // Skip the discriminator
        let mut offset = 0;

        offset += 1;

        let mut amm_config = [0u8; 32];
        amm_config.copy_from_slice(&data[offset..offset + 32]);
        let amm_config = Pubkey::new_from_array(amm_config);
        offset += 32;

        offset += 32;

        let mut token_mint_0 = [0u8; 32];
        token_mint_0.copy_from_slice(&data[offset..offset + 32]);
        let token_mint_0 = Pubkey::new_from_array(token_mint_0);
        offset += 32;

        let mut token_mint_1 = [0u8; 32];
        token_mint_1.copy_from_slice(&data[offset..offset + 32]);
        let token_mint_1 = Pubkey::new_from_array(token_mint_1);
        offset += 32;

        let mut token_vault_0 = [0u8; 32];
        token_vault_0.copy_from_slice(&data[offset..offset + 32]);
        let token_vault_0 = Pubkey::new_from_array(token_vault_0);
        offset += 32;

        let mut token_vault_1 = [0u8; 32];
        token_vault_1.copy_from_slice(&data[offset..offset + 32]);
        let token_vault_1 = Pubkey::new_from_array(token_vault_1);
        offset += 32;

        let mut observation_key = [0u8; 32];
        observation_key.copy_from_slice(&data[offset..offset + 32]);
        let observation_key = Pubkey::new_from_array(observation_key);
        offset += 32;

        offset += 2;

        let mut tick_spacing_bytes = [0u8; 2];
        tick_spacing_bytes.copy_from_slice(&data[offset..offset + 2]);
        let tick_spacing = u16::from_le_bytes(tick_spacing_bytes);
        offset += 2;

        offset += 16;

        // Skip sqrt_price_x64
        offset += 16;

        let mut tick_current_bytes = [0u8; 4];
        tick_current_bytes.copy_from_slice(&data[offset..offset + 4]);
        let tick_current = i32::from_le_bytes(tick_current_bytes);
        offset += 4;

        Ok(Self {
            amm_config,
            token_mint_0,
            token_mint_1,
            token_vault_0,
            token_vault_1,
            observation_key,
            tick_spacing,
            tick_current,
            ..Default::default()
        })
    }
}

pub fn compute_tick_array_start_index(tick: i32, tick_spacing: u16) -> i32 {
    let ticks_in_array = TICK_ARRAY_SIZE * tick_spacing as i32;
    let mut start = tick / ticks_in_array;
    if tick < 0 && tick % ticks_in_array != 0 {
        start = start - 1
    }
    start * ticks_in_array
}

pub fn get_tick_array_pubkeys(
    pool_pubkey: &Pubkey,
    tick_current: i32,
    tick_spacing: u16,
    offsets: &[i32],
    raydium_clmm_program_id: &Pubkey,
) -> Result<Vec<Pubkey>> {
    let mut result = Vec::with_capacity(offsets.len());
    let ticks_in_array = TICK_ARRAY_SIZE * tick_spacing as i32;

    for &offset in offsets {
        let base_start_index = compute_tick_array_start_index(tick_current, tick_spacing);

        let offset_start_index = base_start_index + offset * ticks_in_array;

        let seeds = &[
            TICK_ARRAY_SEED.as_bytes(),
            pool_pubkey.as_ref(),
            &offset_start_index.to_be_bytes(),
        ];

        let (pubkey, _) = Pubkey::find_program_address(seeds, raydium_clmm_program_id);
        result.push(pubkey);
    }

    Ok(result)
}

impl<'info> RaydiumSwap<'info> {
    pub fn process_swap(
        &self,
        amount_in: u64,
        minimum_amount_out: u64,
    ) -> Result<()> {
        let ix = Instruction {
            program_id: self.amm_id.key(),
            accounts: vec![
                AccountMeta::new(self.amm_id.key(), false),
                AccountMeta::new(self.amm_authority.key(), false),
                AccountMeta::new(self.amm_open_orders.key(), false),
                AccountMeta::new(self.pool_coin_token_account.key(), false),
                AccountMeta::new(self.pool_pc_token_account.key(), false),
                AccountMeta::new_readonly(self.serum_program_id.key(), false),
                AccountMeta::new(self.serum_market.key(), false),
                AccountMeta::new(self.serum_bids.key(), false),
                AccountMeta::new(self.serum_asks.key(), false),
                AccountMeta::new(self.serum_event_queue.key(), false),
                AccountMeta::new(self.serum_coin_vault_account.key(), false),
                AccountMeta::new(self.serum_pc_vault_account.key(), false),
                AccountMeta::new_readonly(self.serum_vault_signer.key(), false),
                AccountMeta::new(self.user_source_token.key(), false),
                AccountMeta::new(self.user_destination_token.key(), false),
                AccountMeta::new_readonly(self.user_authority.key(), true),
                AccountMeta::new_readonly(self.token_program.key(), false),
            ],
            data: self.build_swap_instruction_data(amount_in, minimum_amount_out),
        };

        invoke(
            &ix,
            &[
                self.amm_id.to_account_info(),
                self.amm_authority.to_account_info(),
                self.amm_open_orders.to_account_info(),
                self.pool_coin_token_account.to_account_info(),
                self.pool_pc_token_account.to_account_info(),
                self.serum_program_id.to_account_info(),
                self.serum_market.to_account_info(),
                self.serum_bids.to_account_info(),
                self.serum_asks.to_account_info(),
                self.serum_event_queue.to_account_info(),
                self.serum_coin_vault_account.to_account_info(),
                self.serum_pc_vault_account.to_account_info(),
                self.serum_vault_signer.to_account_info(),
                self.user_source_token.to_account_info(),
                self.user_destination_token.to_account_info(),
                self.user_authority.to_account_info(),
                self.token_program.to_account_info(),
            ],
        ).map_err(|_| ErrorCode::RaydiumSwapFailed)?;

        Ok(())
    }

    fn build_swap_instruction_data(&self, amount_in: u64, minimum_amount_out: u64) -> Vec<u8> {
        let mut data = Vec::with_capacity(9);
        data.push(9u8); // Swap instruction discriminator
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&minimum_amount_out.to_le_bytes());
        data
    }
}
