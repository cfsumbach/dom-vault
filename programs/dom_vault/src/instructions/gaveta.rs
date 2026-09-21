//! Upgrade J — três instruções pequenas em volta da gaveta e do índice.
//!
//! A gaveta é uma conta de USDC **apontada pela mesa** (D-F2-43 §6b.2, item 4
//! revertido em 21/09): em mainnet, a ATA do vault 1 do Squads (`9ecvK773…`,
//! carteira `2nFNfsWi…`). O programa **só lê** o saldo dela; quem move é a dona,
//! assinando o `deposit_especial`. Nunca é autoridade da conta.
//!
//! `set_gaveta_usdc`     privilegiada (vault 0, 2/3): aponta a gaveta. Só com o
//!                       ciclo sem P absorvido — trocar a gaveta com P dentro
//!                       do índice deixaria ganho creditado sem caixa atrás.
//! `sincronizar_gaveta"  sem privilégio: absorve o P novo no índice AGORA.
//!                       Quem vai transferir cota chama isto na mesma
//!                       transação, para o destinatário entrar com o índice
//!                       em dia (o hook não carrega a gaveta).
//! `abrir_posicao`       sem privilégio: cria a `PosicaoDoCotista` de uma
//!                       carteira da whitelist que ainda não aportou — o hook
//!                       exige que a conta exista para receber transferência.
use {
    crate::{
        constants::*,
        error::DomError,
        state::{PosicaoDoCotista, Vault},
        whitelist::require_whitelisted,
    },
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface},
};

#[derive(Accounts)]
pub struct SetGavetaUsdc<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Box<Account<'info, Vault>>,
    /// A gaveta nova: conta de USDC de qualquer dona — a dona é quem vai
    /// assinar o fechamento. Não pode ser a treasury (o P sairia do caixa
    /// para o caixa e o índice contaria o cofre inteiro como lucro).
    #[account(
        token::mint = vault.usdc_mint,
        constraint = gaveta.key() != vault.treasury @ DomError::GavetaErrada,
    )]
    pub gaveta: Box<InterfaceAccount<'info, TokenAccount>>,
}

pub fn handle_set_gaveta_usdc(ctx: Context<SetGavetaUsdc>) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    // Só entre ciclos: com P já absorvido, o `p_ciclo` é da conta antiga e o
    // fechamento exigiria esse P na conta nova.
    require!(vault.p_ciclo == 0, DomError::PConservacaoFalhou);
    vault.gaveta_usdc = ctx.accounts.gaveta.key();
    // O saldo que a conta JA' TEM ao ser apontada nao e' P: e' o ponto zero.
    // So' o que entrar depois vira P (mesa, 21/09). Sem isto, apontar uma
    // conta com saldo creditaria ganho de um dinheiro que ninguem mandou como
    // lucro deste ciclo.
    vault.gaveta_saldo_visto = ctx.accounts.gaveta.amount;
    msg!("gaveta apontada: {}", vault.gaveta_usdc);
    Ok(())
}

#[derive(Accounts)]
pub struct SincronizarGaveta<'info> {
    #[account(mut, seeds = [VAULT_SEED], bump = vault.bump)]
    pub vault: Box<Account<'info, Vault>>,
    #[account(constraint = gaveta.key() == vault.gaveta_usdc @ DomError::GavetaErrada)]
    pub gaveta: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = vault.dom_mint @ DomError::UnknownMint)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
}

pub fn handle_sincronizar_gaveta(ctx: Context<SincronizarGaveta>) -> Result<()> {
    let supply = ctx.accounts.dom_mint.supply;
    let saldo = ctx.accounts.gaveta.amount;
    let key = ctx.accounts.gaveta.key();
    crate::indice::absorver_no_cofre(&mut ctx.accounts.vault, &key, saldo, supply)?;
    Ok(())
}

#[derive(Accounts)]
pub struct AbrirPosicao<'info> {
    #[account(mut)]
    pub pagador: Signer<'info>,
    /// CHECK: so' identidade; a whitelist dela e' conferida abaixo.
    pub owner: UncheckedAccount<'info>,
    /// CHECK: validada por `require_whitelisted`.
    pub owner_whitelist: UncheckedAccount<'info>,
    #[account(seeds = [VAULT_SEED], bump = vault.bump)]
    pub vault: Box<Account<'info, Vault>>,
    #[account(
        init,
        payer = pagador,
        space = 8 + PosicaoDoCotista::INIT_SPACE,
        seeds = [POSICAO_SEED, owner.key().as_ref()],
        bump,
    )]
    pub posicao: Box<Account<'info, PosicaoDoCotista>>,
    /// A ATA de cota do dono, quando existe (carteira da era I com cota, na
    /// migração). Sem ela — carteira nova, sem cota — passa o programa como
    /// conta nula. Com cota, a entrada nasce em `indice_ciclo` (ver o handler).
    #[account(
        token::mint = vault.dom_mint,
        token::authority = owner,
        token::token_program = dom_token_program,
    )]
    pub owner_dom: Option<Box<InterfaceAccount<'info, TokenAccount>>>,
    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_abrir_posicao(ctx: Context<AbrirPosicao>) -> Result<()> {
    let owner = ctx.accounts.owner.key();
    // Socio nao passa pela whitelist (recebe cota por cunhagem, nunca pelo
    // hook) — mas precisa da posicao para o fechamento acertar a conta dele.
    if !ctx.accounts.vault.socios.contains(&owner) {
        require_whitelisted(&ctx.accounts.owner_whitelist, &owner)?;
    }
    // Cota ja' existente (era I): a carteira estava dentro ANTES de qualquer
    // P deste ciclo, entao entra no piso do ciclo — `indice_ciclo` —, nao no
    // indice de agora. Sem isso, abrir a posicao de um holder DEPOIS de um P
    // absorvido o faria perder esse P (ameaca "abrir posicao alheia depois
    // de P": qualquer pagador pode abrir a posicao de qualquer carteira).
    // Sem cota, a entrada nao importa: o primeiro lote a define pela media
    // ponderada com cotas_antes = 0.
    let cotas = match ctx.accounts.owner_dom.as_ref() {
        Some(conta) => {
            crate::indice::exigir_ata(
                &conta.key(),
                &owner,
                &ctx.accounts.vault.dom_mint,
                ctx.accounts.dom_token_program.key,
            )?;
            conta.amount
        }
        None => 0,
    };
    let v = &ctx.accounts.vault;
    let p = &mut ctx.accounts.posicao;
    p.owner = owner;
    p.bump = ctx.bumps.posicao;
    p.indice_entrada = if cotas > 0 {
        v.indice_ciclo
    } else {
        v.indice_p
    };
    // nasce acertada: nada pago, nada devido ate' aqui
    p.indice_diluicao_visto = v.indice_diluicao;
    p.indice_p_acertado = v.indice_ciclo;
    Ok(())
}
