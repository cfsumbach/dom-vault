use {
    crate::{constants::*, error::DomError, events::PauseToggled, state::Vault},
    anchor_lang::prelude::*,
};

/// `pause` / `unpause`.
///
/// Bloqueia **entrada e pedido novo** — `deposit`, `request_redeem`,
/// `accrue_performance`, `redeem_fee_share`. **Não bloqueia**
/// `process_redemptions`: a fila que já existe continua sendo honrada (M9/T48).
///
/// O racional está no R1 do runbook: pausa é o análogo on-chain da desmontagem.
/// Quem já pediu resgate antes da crise não pode ser punido por ela.
#[derive(Accounts)]
pub struct SetPause<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
}

pub fn handle_pause(ctx: Context<SetPause>) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    require!(!vault.paused, DomError::Paused);
    vault.paused = true;

    emit!(PauseToggled {
        paused: true,
        authority: vault.authority,
    });
    Ok(())
}

pub fn handle_unpause(ctx: Context<SetPause>) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    require!(vault.paused, DomError::NotPaused);
    vault.paused = false;

    emit!(PauseToggled {
        paused: false,
        authority: vault.authority,
    });
    Ok(())
}
