use {
    crate::{constants::*, error::DomError, events::CapEnforcementEnabled, state::Vault},
    anchor_lang::prelude::*,
};

/// Ativa o cap de 25%. **One-way** (D3): não existe instrução que devolva
/// `cap_enforced` para `false`. O critério de ativação é operacional, não
/// on-chain — ver R2 do runbook.
#[derive(Accounts)]
pub struct EnableCap<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
}

pub fn handle_enable_cap(ctx: Context<EnableCap>) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    require!(!vault.cap_enforced, DomError::CapAlreadyEnforced);

    vault.cap_enforced = true;

    emit!(CapEnforcementEnabled {
        authority: vault.authority,
    });
    Ok(())
}
