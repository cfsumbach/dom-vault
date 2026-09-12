use {
    crate::{constants::*, error::DomError, events::SociosUpdated, state::Vault},
    anchor_lang::prelude::*,
};

/// Atualiza as três carteiras de sócio.
///
/// **A mesma trava do `initialize`** (D10): distintas par a par. O mandato §1.2
/// diz que os endereços são configuráveis, logo existe este caminho — e sem a
/// checagem aqui a validação do `initialize` seria contornável em dois passos:
/// configura distinto, atualiza para duplicado.
#[derive(Accounts)]
pub struct UpdateSocios<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
}

/// Distinção par a par. Com três elementos são três comparações — explícito é
/// melhor que laço aqui, porque a lista não cresce (o 20/20/20 pressupõe três).
pub fn require_socios_distintos(socios: &[Pubkey; NUM_SOCIOS]) -> Result<()> {
    require_keys_neq!(socios[0], socios[1], DomError::DuplicateSocio);
    require_keys_neq!(socios[0], socios[2], DomError::DuplicateSocio);
    require_keys_neq!(socios[1], socios[2], DomError::DuplicateSocio);
    Ok(())
}

pub fn handle_update_socios(
    ctx: Context<UpdateSocios>,
    socios: [Pubkey; NUM_SOCIOS],
) -> Result<()> {
    require_socios_distintos(&socios)?;

    ctx.accounts.vault.socios = socios;

    emit!(SociosUpdated { socios });
    Ok(())
}
