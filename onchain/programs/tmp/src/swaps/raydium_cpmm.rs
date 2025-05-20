use anchor_lang::prelude::*;
use anchor_lang::solana_program::{instruction::Instruction, program::invoke};
use anchor_spl::token::{Token, TokenAccount};

use crate::error::ErrorCode;
use crate::state::RaydiumSwapState;
const AMM_CONFIG_OFFSET: usize = 8; // amm_config
const POOL_CREATOR_OFFSET: usize = 40; // pool_creator
const TOKEN_0_VAULT_OFFSET: usize = 72; // token_0_vault
const TOKEN_1_VAULT_OFFSET: usize = 104; // token_1_vault
const LP_MINT_OFFSET: usize = 136; // lp_mint
const TOKEN_0_MINT_OFFSET: usize = 168; // token_0_mint
const TOKEN_1_MINT_OFFSET: usize = 200; // token_1_mint
const TOKEN_0_PROGRAM_OFFSET: usize = 232; // token_0_program
const TOKEN_1_PROGRAM_OFFSET: usize = 264; // token_1_program
const OBSERVATION_KEY_OFFSET: usize = 296; // observation_key

#[derive(Debug)]
pub struct RaydiumCpAmmInfo {
    pub token_0_mint: Pubkey,
    pub token_1_mint: Pubkey,
    pub token_0_vault: Pubkey,
    pub token_1_vault: Pubkey,
    pub amm_config: Pubkey,
    pub observation_key: Pubkey,
}

impl RaydiumCpAmmInfo {
    pub fn load_checked(data: &[u8]) -> Result<Self> {
        if data.len() < OBSERVATION_KEY_OFFSET + 32 {
            return Err(anyhow::anyhow!("Invalid data length for RaydiumCpAmmInfo"));
        }
        
        let token_0_vault = Pubkey::new(&data[TOKEN_0_VAULT_OFFSET..TOKEN_0_VAULT_OFFSET + 32]);
        let token_1_vault = Pubkey::new(&data[TOKEN_1_VAULT_OFFSET..TOKEN_1_VAULT_OFFSET + 32]);
        let token_0_mint = Pubkey::new(&data[TOKEN_0_MINT_OFFSET..TOKEN_0_MINT_OFFSET + 32]);
        let token_1_mint = Pubkey::new(&data[TOKEN_1_MINT_OFFSET..TOKEN_1_MINT_OFFSET + 32]);
        let amm_config = Pubkey::new(&data[AMM_CONFIG_OFFSET..AMM_CONFIG_OFFSET + 32]);
        let observation_key = Pubkey::new(&data[OBSERVATION_KEY_OFFSET..OBSERVATION_KEY_OFFSET + 32]);
        
        Ok(Self {
            token_0_mint,
            token_1_mint,
            token_0_vault,
            token_1_vault,
            amm_config,
            observation_key,
        })
    }
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
