use {
    crate::{constants::*, error::DomError, events::DeployAllowlistUpdated, state::Vault},
    anchor_lang::prelude::*,
};

/// Registra (ou limpa) um destino operacional da allowlist do `deploy_capital`.
///
/// **Privilegiada**: só a `authority`, que pela D2 é o Vault PDA do Squads —
/// portanto proposta com quorum. É a instrução que decide para onde o capital
/// dos cotistas pode sair, então não existe caminho de uma assinatura só.
///
/// `Pubkey::default()` **limpa** a vaga. Vaga limpa nunca casa com destino
/// nenhum: `deploy_capital` recusa o zero explicitamente, e conta de token não
/// tem owner zerado de qualquer forma.
#[derive(Accounts)]
pub struct UpdateDeployAllowlist<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
}

pub fn handle_update_deploy_allowlist(
    ctx: Context<UpdateDeployAllowlist>,
    indice: u8,
    destino: Pubkey,
) -> Result<()> {
    let i = indice as usize;
    require!(i < DEPLOY_ALLOWLIST_LEN, DomError::AllowlistIndexOutOfRange);

    // -----------------------------------------------------------------------
    // Carteira de sócio **nunca** entra na allowlist (D-F2-01).
    // -----------------------------------------------------------------------
    // Dinheiro de investidor circula pelos destinos operacionais; dinheiro de
    // sócio sai só por `redeem_fee_share`, contra o livro de `fee_share`. Os
    // dois fluxos não se cruzam no mesmo endereço — e essa separação vale mais
    // como trava do que como parágrafo de documento.
    //
    // A trava não é decorativa: sem ela, uma proposta aprovada distraidamente
    // transformaria `deploy_capital` num caminho de pagamento a sócio que não
    // passa pelo HWM nem pelo livro.
    // -----------------------------------------------------------------------
    if destino != Pubkey::default() {
        require!(
            !ctx.accounts.vault.socios.contains(&destino),
            DomError::SocioNaoPodeSerDestino
        );
    }

    let anterior = ctx.accounts.vault.deploy_allowlist[i];
    ctx.accounts.vault.deploy_allowlist[i] = destino;

    emit!(DeployAllowlistUpdated {
        indice,
        destino,
        anterior,
    });

    Ok(())
}
