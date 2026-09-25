//! `ajustar_deployed_usdc` — a correção do custo de capital em campo.
//! **Privilegiada** (Upgrade K, D-F2-45).
//!
//! ─── por que ela existe, e por que nasce SEM USO ────────────────────────────
//! `deployed_usdc` é o custo do capital que saiu do cofre para o campo, e ele
//! só anda por `deploy_capital` (+) e `return_capital` (−). Isso basta para a
//! operação normal. O que não basta é para o dia em que a contabilidade errar
//! de verdade — e, desde a D-F2-45, esse campo deixou de ser só informação:
//! ele entra no `nav_piso`, e o piso **precifica o aporte**.
//!
//! A `corrigir_deployed_usdc` do Upgrade F resolvia isso exigindo
//! `supply == 0` (D-F2-26) — janela que não volta com investidor externo
//! dentro. Esta a substitui, e a trava passa a ser outra: **só reduz**.
//!
//! ─── só reduz, e a razão é o preço ─────────────────────────────────────────
//! Aumentar `deployed_usdc` levanta o piso, e o piso é o preço mínimo de
//! entrada: seria uma caneta capaz de **encarecer a cota do cotista novo** por
//! decisão de mesa. Reduzir só pode baratear — e quem é diluído por um piso
//! baixo demais é quem já está dentro, que é quem vota. O erro que a trava
//! deixa passar é o erro que se paga em casa.
//!
//! O motivo vai DENTRO da instrução, não só no memo do Squads: o memo é do
//! sistema de propostas, e some com ele; o evento fica na cadeia.
use {
    crate::{constants::*, error::DomError, events::DeployedAjustado, state::Vault},
    anchor_lang::prelude::*,
};

/// Limites do motivo: curto demais não explica, longo demais é log.
pub const MOTIVO_MIN: usize = 8;
pub const MOTIVO_MAX: usize = 200;

#[derive(Accounts)]
pub struct AjustarDeployedUsdc<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
}

pub fn handle_ajustar_deployed_usdc(
    ctx: Context<AjustarDeployedUsdc>,
    novo: u64,
    motivo: String,
) -> Result<()> {
    require!(
        motivo.len() >= MOTIVO_MIN && motivo.len() <= MOTIVO_MAX,
        DomError::MotivoInvalido
    );
    let antes = ctx.accounts.vault.deployed_usdc;
    require!(novo < antes, DomError::DeployedSoReduz);

    ctx.accounts.vault.deployed_usdc = novo;

    emit!(DeployedAjustado {
        antes,
        depois: novo,
        motivo,
        timestamp: Clock::get()?.unix_timestamp,
    });
    Ok(())
}
