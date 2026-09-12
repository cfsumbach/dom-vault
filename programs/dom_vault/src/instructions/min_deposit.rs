use {
    crate::{constants::*, error::DomError, events::MinDepositUpdated, state::Vault},
    anchor_lang::prelude::*,
};

/// Ajusta o depósito mínimo em vigor. **Privilegiada.**
///
/// É a 12ª caneta de governança, e a mesa aceitou o custo com o olho aberto: o
/// mínimo de aporte é parâmetro **operacional** — a estratégia é começar alto,
/// para migrar investidor sem fuga de capital, e baixar em etapas conforme
/// democratiza. Como constante, cada degrau custaria um upgrade de programa;
/// como campo, custa uma proposta.
///
/// Note que a diferença **não é de governança**: upgrade também é proposta
/// 2-de-N, porque a upgrade authority é o Squads. A diferença é fricção — build,
/// buffer, cerimônia — e é isso que o campo compra.
#[derive(Accounts)]
pub struct UpdateMinDeposit<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
}

pub fn handle_update_min_deposit(ctx: Context<UpdateMinDeposit>, novo: u64) -> Result<()> {
    // Zero abriria aporte de poeira e encheria a whitelist de contas de token
    // que não compram nem uma unidade de cota. O piso do piso é 1 micro-USDC.
    require!(novo > 0, DomError::InvalidMinDeposit);

    let anterior = ctx.accounts.vault.min_deposit;
    ctx.accounts.vault.min_deposit = novo;

    emit!(MinDepositUpdated { anterior, novo });
    Ok(())
}
