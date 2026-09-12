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

/// Contas em `Box` pelo mesmo motivo do `Initialize`: dois mints e três contas
/// de token não cabem no frame de 4 KB do BPF. O compilador avisa
/// ("Stack offset ... exceeded max offset") e o programa quebra em runtime com
/// `Access violation` — não é advertência cosmética.
#[derive(Accounts)]
pub struct Deposit<'info> {
    pub depositor: Signer<'info>,
    #[account(
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = dom_mint @ DomError::UnknownMint,
        has_one = usdc_mint @ DomError::UnknownUsdcMint,
        has_one = treasury @ DomError::InvalidTreasury,
        constraint = !vault.paused @ DomError::Paused,
    )]
    pub vault: Box<Account<'info, Vault>>,
    /// CHECK: validada por `require_whitelisted` — ver `whitelist.rs`.
    pub depositor_whitelist: UncheckedAccount<'info>,

    #[account(mut)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = usdc_mint,
        token::authority = depositor,
        token::token_program = usdc_token_program,
    )]
    pub depositor_usdc: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = dom_mint,
        token::authority = depositor,
        token::token_program = dom_token_program,
    )]
    pub depositor_dom: Box<InterfaceAccount<'info, TokenAccount>>,

    /// O DOM é Token-2022 e ponto: é onde o hook existe.
    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
    /// O USDC pode estar sob qualquer um dos dois programas de token — em
    /// devnet costuma ser o clássico.
    pub usdc_token_program: Interface<'info, TokenInterface>,
}

pub fn handle_deposit(ctx: Context<Deposit>, usdc_amount: u64) -> Result<()> {
    let depositor = ctx.accounts.depositor.key();

    // Whitelist antes de qualquer movimento de valor (T05). Mesmo caminho
    // fail-closed do hook — a função é uma só (`whitelist.rs`).
    let piso_proprio = require_whitelisted(&ctx.accounts.depositor_whitelist, &depositor)?;

    // Limite inclusivo: 200,000000 entra, 199,999999 não (T03/T04).
    // O piso vem do ESTADO, nao da constante: a mesa ajusta por proposta
    // conforme democratiza o acesso, sem redeploy.
    // -----------------------------------------------------------------------
    // D-F2-30 — O PISO PODE SER DESTA CARTEIRA, E NAO O DO COFRE.
    //
    // `min_deposit_proprio == 0` significa "sem excecao": vale o piso do cofre,
    // que e' o comportamento de sempre. Entrada de whitelist anterior ao Upgrade
    // G tem 42 bytes, nao carrega o campo, e le' como 0 — entao nada muda para
    // ninguem ate' a mesa decidir mudar, carteira a carteira, por proposta 2/3.
    //
    // E' MECANISMO, NAO POLITICA. Nao ha' regra automatica aqui de proposito: se
    // um dia houver ("quem ja' e' cotista aporta a partir de X"), ela nasce em
    // cima deste campo, e a mesa a decide com casos reais na mao em vez de no
    // papel. O campo sem a regra serve; a regra sem o campo nao existe.
    // -----------------------------------------------------------------------
    let piso = if piso_proprio > 0 {
        piso_proprio
    } else {
        ctx.accounts.vault.min_deposit
    };
    require!(usdc_amount >= piso, DomError::DepositBelowMinimum);

    // DV11 — o depósito é o ponto onde NAV errado vira cota errada, e cota
    // errada dilui quem já está dentro. Primeira leitura a entrar na trava.
    //
    // E antes do staleness, o canto de gênese, fechado por decisão da mesa em
    // 2026-08-20: **cofre sem NAV publicado não aceita aporte**. O NAV de
    // fundação (1,000000) não é preço de oráculo — é preço de partida, e o
    // `staleness` não tem o que medir contra ele. Sem esta trava, um cofre onde
    // o oráculo nunca publicou aceitaria depósito ao preço de partida para
    // sempre. Era o único canto do DV11 sem cobertura.
    // -----------------------------------------------------------------------
    // D-F2-09 — **entrada fechada enquanto a janela de saque de lucro está
    // aberta.**
    //
    // O direito ao saque é `cotas × delta`, e `cotas` é lido no instante do
    // saque. Sem esta trava, quem aportasse depois da distribuição compraria
    // cota ao NAV já subido e ainda sacaria lucro que não gerou — pagando duas
    // vezes o mesmo dinheiro, uma no preço da cota e outra no bolo.
    //
    // A janela é curta por construção: abre no `deposit_especial` e fecha no
    // `deploy_capital` do ciclo seguinte. `lucro_sacavel_restante > 0` **é** a
    // janela — um campo, três serviços.
    // -----------------------------------------------------------------------
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
    // Aporte que, ao NAV corrente, não compra nem uma unidade de cota seria
    // doação ao cofre. Com o mínimo de 200 USDC isso exige NAV absurdo, mas a
    // trava é barata e o caso é o do T51.
    require!(shares > 0, DomError::ZeroShares);

    // USDC do cotista para o caixa do fundo.
    transfer_checked(
        CpiContext::new(
            ctx.accounts.usdc_token_program.key(),
            TransferChecked {
                from: ctx.accounts.depositor_usdc.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.depositor.to_account_info(),
            },
        ),
        usdc_amount,
        ctx.accounts.usdc_mint.decimals,
    )?;

    // Cotas para o cotista. Quem assina é o PDA do cofre (D15).
    let vault_bump = ctx.accounts.vault.bump;
    let vault_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, &[vault_bump]]];
    mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.dom_token_program.key(),
            MintTo {
                mint: ctx.accounts.dom_mint.to_account_info(),
                to: ctx.accounts.depositor_dom.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            vault_seeds,
        ),
        shares,
    )?;

    // -----------------------------------------------------------------------
    // Cap de 25% no caminho de mint (D3: cap é invariante de supply, checado em
    // transfer E mint) — T17.
    //
    // `reload` em vez de somar `shares` à mão: o saldo e o supply pós-mint são
    // lidos das contas, mesma disciplina da D4 no hook. Somar à mão é como se
    // erra por double-count quando um dia entrar taxa ou desconto no caminho.
    //
    // A rejeição aqui reverte a transação inteira, inclusive o USDC já
    // transferido — é atômico, não fica meio depósito.
    // -----------------------------------------------------------------------
    if ctx.accounts.vault.cap_enforced {
        ctx.accounts.dom_mint.reload()?;
        ctx.accounts.depositor_dom.reload()?;

        let balance = ctx.accounts.depositor_dom.amount as u128;
        let supply = ctx.accounts.dom_mint.supply as u128;
        require!(
            balance.saturating_mul(100)
                <= supply.saturating_mul(ctx.accounts.vault.cap_pct as u128),
            DomError::CapExceeded
        );
    }

    emit!(Deposited {
        depositor,
        usdc: usdc_amount,
        shares,
        nav,
    });

    Ok(())
}
