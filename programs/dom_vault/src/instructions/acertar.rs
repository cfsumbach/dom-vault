//! J7 — `acertar(carteira)`: a taxa da mesa cobrada pelo ÍNDICE, não pela diluição.
//!
//! No fechamento a mesa recebe `taxa_mesa × P` em cotas — diluição uniforme, que
//! cobra cada carteira pelo TAMANHO. A regra (D-F2-43 §2) cobra pelo GANHO. Esta
//! instrução acerta a diferença, carteira a carteira, quando alguém a toca:
//!
//!   devido = taxa_mesa × ganho nos ciclos já fechados desde o último acerto
//!   pago   = cotas × (indice_diluicao − indice_diluicao_visto)
//!   pago > devido  → mint_to(carteira, (pago − devido) ÷ nav_fechamento)   — CRÉDITO, sem privilégio
//!   pago < devido  → burn(carteira,    (devido − pago) ÷ nav_fechamento)   — DÉBITO, o dono assina
//!
//! **O contrato não queima cota de ninguém sem assinatura** (o mint não tem
//! `PermanentDelegate`). Por isso o débito exige o dono; e toda SAÍDA de cota
//! (resgate, saque de lucro, transferência) exige a carteira acertada — o dono
//! assina a saída, e a dívida é cobrada nela. Carteira que dorme não paga até
//! acordar; o débito fica registrado nos odômetros. Idempotente: tocar duas
//! vezes no mesmo ciclo é zero. N fechamentos sem toque = um acerto acumulado.
//! O `nav_fechamento` usado é o do último fechamento (segunda ordem quando a
//! carteira atravessou vários — quantificado na prova).
use {
    crate::{
        constants::*,
        error::DomError,
        state::{PosicaoDoCotista, Vault},
    },
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface},
};

#[derive(Accounts)]
pub struct Acertar<'info> {
    /// CHECK: o dono da carteira. Assina SO' quando ha' debito (queima).
    pub owner: UncheckedAccount<'info>,
    #[account(mut, seeds = [VAULT_SEED], bump = vault.bump, has_one = dom_mint @ DomError::UnknownMint)]
    pub vault: Box<Account<'info, Vault>>,
    #[account(mut, seeds = [POSICAO_SEED, owner.key().as_ref()], bump = posicao.bump, constraint = posicao.owner == owner.key() @ DomError::PosicaoDoCotistaAusente)]
    pub posicao: Box<Account<'info, PosicaoDoCotista>>,
    #[account(mut)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, token::mint = dom_mint, token::authority = owner, token::token_program = dom_token_program)]
    pub owner_dom: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
}

pub fn handle_acertar(ctx: Context<Acertar>) -> Result<()> {
    let assina = ctx.accounts.owner.is_signer;
    let owner = ctx.accounts.owner.to_account_info();
    crate::indice::acertar_com_cpi(
        &mut ctx.accounts.vault,
        &mut ctx.accounts.posicao,
        &ctx.accounts.owner_dom,
        &ctx.accounts.dom_mint,
        &owner,
        assina,
        &ctx.accounts.dom_token_program,
    )
}
