//! **Aporte EM NOME DE OUTRA CARTEIRA — o bônus da mesa em uma assinatura.**
//!
//! A mesa quis dar cota de bônus a membros da comunidade. Isso já funcionava em
//! **duas** transações — a mesa aporta e depois transfere a cota —, e o caminho
//! continua válido. Esta instrução faz o mesmo em **uma**, atomicamente: não
//! existe o estado intermediário em que o aporte passou e a transferência não.
//!
//! # O que ela NÃO é
//!
//! **Não é emissão sem lastro.** O USDC entra no caixa antes da cota nascer,
//! como no `deposit` — a diferença é só que quem paga e quem recebe são
//! carteiras distintas. Bônus de 100 USDC custa 100 USDC a quem o dá.
//!
//! # ⚠️ A TRAVA QUE FAZ O RESTO DO DESENHO FECHAR
//!
//! **`mint_to` NÃO dispara o transfer hook.** O hook roda em transferência, e
//! aqui a cota **nasce** na conta do beneficiário.
//!
//! Sem conferir a whitelist do beneficiário explicitamente, esta instrução seria
//! uma porta para cunhar cota em qualquer carteira do mundo — furando de uma vez
//! a autoridade de congelamento, o hook e a whitelist, que são o fundo fechado
//! inteiro. Não seria um defeito: seria um buraco projetado.
//!
//! **As DUAS pontas são conferidas**: quem paga e quem recebe. Dinheiro entrando
//! de carteira que a mesa não aprovou é um canal que ela não abriu — mesmo que a
//! cota vá para alguém aprovado.
//!
//! # De quem é o piso mínimo
//!
//! **Do beneficiário.** Quem está entrando é ele, e é a ficha dele que carrega a
//! exceção da `D-F2-30`.

use {
    crate::{
        constants::*,
        error::DomError,
        events::Deposited,
        math::{require_nav_fresco, shares_from_usdc},
        state::Vault,
        whitelist::require_whitelisted,
    },
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{
        mint_to, transfer_checked, Mint, MintTo, TokenAccount, TokenInterface, TransferChecked,
    },
};

#[derive(Accounts)]
pub struct DepositPara<'info> {
    /// Quem PAGA o USDC. Assina, e não recebe cota nenhuma.
    pub aportador: Signer<'info>,
    /// CHECK: validada por `require_whitelisted`.
    pub aportador_whitelist: UncheckedAccount<'info>,

    /// Quem RECEBE a cota. Só o endereço — não assina.
    /// CHECK: só identidade; a aprovação é conferida na `beneficiario_whitelist`.
    pub beneficiario: UncheckedAccount<'info>,
    /// CHECK: validada por `require_whitelisted`. **É a trava do cabeçalho.**
    pub beneficiario_whitelist: UncheckedAccount<'info>,

    #[account(
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = dom_mint @ DomError::UnknownMint,
        has_one = usdc_mint @ DomError::UnknownUsdcMint,
        has_one = treasury @ DomError::InvalidTreasury,
        constraint = !vault.paused @ DomError::Paused,
    )]
    pub vault: Box<Account<'info, Vault>>,

    #[account(mut)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = usdc_mint,
        token::authority = aportador,
        token::token_program = usdc_token_program,
    )]
    pub aportador_usdc: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = dom_mint,
        token::authority = beneficiario,
        token::token_program = dom_token_program,
    )]
    pub beneficiario_dom: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
    pub usdc_token_program: Interface<'info, TokenInterface>,
}

pub fn handle_deposit_para(ctx: Context<DepositPara>, usdc_amount: u64) -> Result<()> {
    let aportador = ctx.accounts.aportador.key();
    let beneficiario = ctx.accounts.beneficiario.key();

    // AS DUAS PONTAS. Ver o cabeçalho: sem a segunda, isto vira porta.
    require_whitelisted(&ctx.accounts.aportador_whitelist, &aportador)?;
    let piso_proprio = require_whitelisted(&ctx.accounts.beneficiario_whitelist, &beneficiario)?;

    // O piso é o DO BENEFICIÁRIO — quem entra é ele (`D-F2-30`).
    let piso = if piso_proprio > 0 {
        piso_proprio
    } else {
        ctx.accounts.vault.min_deposit
    };
    require!(usdc_amount >= piso, DomError::DepositBelowMinimum);

    // A janela de lucro fecha a entrada, aqui como no `deposit` e pelo mesmo
    // motivo (D-F2-09): quem entra na janela levaria lucro que não gerou.
    require!(
        ctx.accounts.vault.lucro_sacavel_restante == 0,
        DomError::JanelaDeLucroAberta
    );

    let agora = Clock::get()?.unix_timestamp;
    require!(ctx.accounts.vault.nav_ts != 0, DomError::NavNeverPublished);
    require_nav_fresco(
        ctx.accounts.vault.nav_ts,
        agora,
        ctx.accounts.vault.max_nav_staleness,
    )?;

    let nav = ctx.accounts.vault.nav;
    let shares = shares_from_usdc(usdc_amount, nav)?;
    require!(shares > 0, DomError::ZeroShares);

    // O DINHEIRO ENTRA ANTES DA COTA NASCER. Se a transferência falhar, a
    // transação inteira reverte e nenhuma cota foi emitida contra lucro que não
    // chegou — mesma ordem deliberada do `deposit`.
    transfer_checked(
        CpiContext::new(
            ctx.accounts.usdc_token_program.key(),
            TransferChecked {
                from: ctx.accounts.aportador_usdc.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.aportador.to_account_info(),
            },
        ),
        usdc_amount,
        ctx.accounts.usdc_mint.decimals,
    )?;

    let vault_bump = ctx.accounts.vault.bump;
    let vault_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, &[vault_bump]]];
    mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.dom_token_program.key(),
            MintTo {
                mint: ctx.accounts.dom_mint.to_account_info(),
                to: ctx.accounts.beneficiario_dom.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            vault_seeds,
        ),
        shares,
    )?;

    // O cap olha o BENEFICIÁRIO, que é quem ficou com a cota.
    if ctx.accounts.vault.cap_enforced {
        ctx.accounts.dom_mint.reload()?;
        ctx.accounts.beneficiario_dom.reload()?;
        let balance = ctx.accounts.beneficiario_dom.amount as u128;
        let supply = ctx.accounts.dom_mint.supply as u128;
        require!(
            balance.saturating_mul(100) <= supply.saturating_mul(ctx.accounts.vault.cap_pct as u128),
            DomError::CapExceeded
        );
    }

    // O evento registra o BENEFICIÁRIO no campo `depositor`: quem recebeu a cota
    // é quem o indexador precisa ver. Quem pagou está na transação, assinando.
    emit!(Deposited {
        depositor: beneficiario,
        usdc: usdc_amount,
        shares,
        nav,
    });
    Ok(())
}
