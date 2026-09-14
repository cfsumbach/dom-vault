//! **Nomear e destituir o porteiro da whitelist — `D-F2-34`.**
//!
//! Aprovar cotista era uma proposta 2/3 **por pessoa**. O gargalo nunca foi a
//! decisão: a mesa já decide na fila do painel. Era a assinatura de hardware
//! para executar o que já tinha sido decidido — e com fila de espera, isso é o
//! que trava a captação.
//!
//! ⚠️ **A NOMEAÇÃO CONTINUA SENDO 2/3.** Só o ato repetido sai da mesa; o poder
//! de delegar, não. E `Pubkey::default()` destitui — não é preciso fechar conta
//! nem inventar uma instrução de revogação, que é a que ninguém testa.

use {
    crate::{constants::*, error::DomError, state::*},
    anchor_lang::prelude::*,
};

#[event]
pub struct PorteiroDaWhitelistTrocado {
    pub anterior: Pubkey,
    pub novo: Pubkey,
}

#[derive(Accounts)]
pub struct SetWhitelistOperator<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// **Só a mesa.** Delegar é ato dela, e continua sendo 2/3.
    pub authority: Signer<'info>,
    #[account(
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + WhitelistOperator::INIT_SPACE,
        seeds = [WL_OPERATOR_SEED],
        bump,
    )]
    pub wl_operator: Account<'info, WhitelistOperator>,
    pub system_program: Program<'info, System>,
}

pub fn handle_set_whitelist_operator(
    ctx: Context<SetWhitelistOperator>,
    novo: Pubkey,
) -> Result<()> {
    let anterior = ctx.accounts.wl_operator.operator;
    ctx.accounts.wl_operator.operator = novo;
    ctx.accounts.wl_operator.bump = ctx.bumps.wl_operator;

    emit!(PorteiroDaWhitelistTrocado { anterior, novo });
    Ok(())
}
