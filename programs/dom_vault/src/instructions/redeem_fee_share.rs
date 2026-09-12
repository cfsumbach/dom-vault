use {
    crate::{
        constants::*,
        error::DomError,
        events::FeeShareRedeemed,
        math::{require_nav_fresco, usdc_from_shares},
        state::{FeeShareLedger, Vault},
    },
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{
        burn, transfer_checked, Burn, Mint, TokenAccount, TokenInterface, TransferChecked,
    },
};

/// Resgate de `fee_share`: **instantâneo**, contra caixa livre, sem fila D+30.
///
/// A trava que separa isto do resgate comum não está no token — `fee_share` e
/// cota comum são o mesmo mint — e sim no livro por sócio (D3.1). Quem não tem
/// livro não resgata; quem tem, resgata só até o saldo do livro. É o que faz
/// T44 e T45 caírem: cota comum, inclusive a de sócio, segue a fila.
#[derive(Accounts)]
pub struct RedeemFeeShare<'info> {
    pub socio: Signer<'info>,
    #[account(
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = dom_mint @ DomError::UnknownMint,
        has_one = usdc_mint @ DomError::UnknownUsdcMint,
        has_one = treasury @ DomError::InvalidTreasury,
        constraint = !vault.paused @ DomError::Paused,
    )]
    pub vault: Box<Account<'info, Vault>>,
    #[account(
        mut,
        seeds = [FEE_SHARE_SEED, socio.key().as_ref()],
        bump = ledger.bump,
        constraint = ledger.socio == socio.key() @ DomError::SocioMismatch,
    )]
    pub ledger: Box<Account<'info, FeeShareLedger>>,

    #[account(mut)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        mut,
        token::mint = dom_mint,
        token::authority = socio,
        token::token_program = dom_token_program,
    )]
    pub socio_dom: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = usdc_mint,
        token::authority = socio,
        token::token_program = usdc_token_program,
    )]
    pub socio_usdc: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
    pub usdc_token_program: Interface<'info, TokenInterface>,
}

pub fn handle_redeem_fee_share(ctx: Context<RedeemFeeShare>, shares: u64) -> Result<()> {
    require!(shares > 0, DomError::ZeroSharesRequested);
    require!(
        ctx.accounts.ledger.shares >= shares,
        DomError::InsufficientFeeShare
    );

    // -----------------------------------------------------------------------
    // NAV velho não paga taxa. (D4, corrigida no Upgrade C — 2026-08-26)
    // -----------------------------------------------------------------------
    // Esta trava **faltava**, e a ausência não era decisão: era esquecimento. Os
    // outros cinco caminhos que transformam NAV em dinheiro — `deposit`,
    // `deposit_especial`, `sacar_lucro`, `deploy_capital` e
    // `efetivar_resgate_capital` — sempre a tiveram.
    //
    // **Este é o caminho de pagamento do sócio**, e era o único de saída de
    // dinheiro sem ela. NAV parado alto pagaria o sócio a mais, do treasury, à
    // custa dos cotistas — e o sócio é quem controla o multisig e o oráculo. A
    // trava faltar exatamente aí é a assimetria que se procura primeiro.
    //
    // Achada em 2026-08-26 conferindo, para o dossiê da auditoria, a afirmação de
    // que a trava cobria todos os caminhos. Cobria cinco de seis.
    // -----------------------------------------------------------------------
    let now = Clock::get()?.unix_timestamp;
    require_nav_fresco(
        ctx.accounts.vault.nav_ts,
        now,
        ctx.accounts.vault.max_nav_staleness,
    )?;

    let nav = ctx.accounts.vault.nav;
    let valor = usdc_from_shares(shares, nav)?;

    // -----------------------------------------------------------------------
    // A reserva de 10% é **piso** aqui (D7/T43) — ao contrário do
    // `process_redemptions`, que pode consumi-la. A reserva existe para honrar
    // a fila; deixar a taxa dos sócios comê-la inverteria a ordem de quem
    // recebe primeiro num aperto de caixa.
    //
    // Sem caixa livre suficiente, o resgate é **bloqueado** e o `fee_share`
    // continua no livro. Não vira pedido, não entra na fila D+30 — fica
    // pendente até haver caixa.
    // -----------------------------------------------------------------------
    let patrimonio = usdc_from_shares(ctx.accounts.dom_mint.supply, nav)?;
    let reserva = u64::try_from(
        (patrimonio as u128)
            .checked_mul(ctx.accounts.vault.reserve_bps as u128)
            .ok_or(DomError::MathOverflow)?
            .checked_div(BPS_DEN)
            .ok_or(DomError::MathOverflow)?,
    )
    .map_err(|_| error!(DomError::MathOverflow))?;
    let livre = ctx.accounts.treasury.amount.saturating_sub(reserva);
    require!(valor <= livre, DomError::NoFreeCash);

    // Queima da cota do sócio. Queima não passa pelo hook (D17).
    burn(
        CpiContext::new(
            ctx.accounts.dom_token_program.key(),
            Burn {
                mint: ctx.accounts.dom_mint.to_account_info(),
                from: ctx.accounts.socio_dom.to_account_info(),
                authority: ctx.accounts.socio.to_account_info(),
            },
        ),
        shares,
    )?;

    let vault_bump = ctx.accounts.vault.bump;
    let vault_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, &[vault_bump]]];
    let usdc_decimals = ctx.accounts.usdc_mint.decimals;
    transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.usdc_token_program.key(),
            TransferChecked {
                from: ctx.accounts.treasury.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.socio_usdc.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            vault_seeds,
        ),
        valor,
        usdc_decimals,
    )?;

    ctx.accounts.ledger.shares -= shares;

    emit!(FeeShareRedeemed {
        socio: ctx.accounts.socio.key(),
        shares,
        usdc: valor,
        nav,
    });

    Ok(())
}
