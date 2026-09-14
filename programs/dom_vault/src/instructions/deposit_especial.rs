use {
    crate::{
        constants::*,
        error::DomError,
        events::{JanelaDeLucroEncerrada, LucroDistribuido},
        math::{require_nav_fresco, shares_from_usdc, usdc_from_shares},
        state::{FeeShareLedger, Vault},
    },
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{
        mint_to, transfer_checked, Mint, MintTo, TokenAccount, TokenInterface, TransferChecked,
    },
};

/// **Deposit especial** — o lucro realizado do ciclo entra no cofre e se
/// reparte 60/40 na mesma transação. **Privilegiada.**
///
/// Substitui o `accrue_performance` (D-F2-09), e a troca é de régua, não de
/// implementação:
///
/// | | `accrue_performance` (fora) | `deposit_especial` (aqui) |
/// |---|---|---|
/// | base da taxa | `nav - hwm` | `P`, o valor transferido nesta instrução |
/// | origem do lucro | qualquer subida de NAV, marcação inclusive | dinheiro que voltou ao caixa |
/// | caixa por trás | nenhum — o cofre já podia estar sem | o próprio `P`, transferido aqui |
///
/// O defeito que isso fecha estava vivo: o `accrue_performance` cobrava 60%
/// sobre valorização de NAV vinda de **marcação**, e o `redeem_fee_share`
/// pagava essa cota em USDC de verdade, na hora, contra a treasury. Marcação
/// virava caixa saindo do cofre. Com a base sendo `P`, a taxa só existe depois
/// que o dinheiro entrou — e ele entra aqui, na mesma assinatura.
///
/// # Por onde o dinheiro chega
///
/// `origem_usdc` é a conta de USDC **da autoridade**, isto é, do Vault PDA do
/// Squads. A mesa devolve o lucro da carteira operacional para lá com uma
/// transferência SPL comum (fora do programa), e a proposta executa esta
/// instrução. São dois passos de propósito: o lucro passa pela custódia do
/// multisig **antes** de ser repartido, e a repartição fica atômica com a
/// entrada do dinheiro. Não existe distribuição declarada sem caixa por trás.
///
/// O **capital** de volta continua entrando pelo `return_capital`, que não é
/// privilegiado. Aqui entra só o que passou do capital: o `P`.
///
/// # A janela de saque
///
/// Esta instrução **abre** a janela: grava `lucro_sacavel_restante = 40% de P`
/// e o `delta_lucro_por_cota`. Quem fecha é o `deploy_capital` do ciclo
/// seguinte — o marco é o re-deploy, não um cronômetro (D-F2-09).
#[derive(Accounts)]
pub struct DepositEspecial<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
        has_one = dom_mint @ DomError::UnknownMint,
        has_one = usdc_mint @ DomError::UnknownUsdcMint,
        has_one = treasury @ DomError::InvalidTreasury,
        constraint = !vault.paused @ DomError::Paused,
    )]
    pub vault: Box<Account<'info, Vault>>,
    #[account(mut)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,
    /// Conta de USDC de onde sai o `P`. Pertence à **autoridade** — o Vault PDA
    /// do Squads —, então quem assina a saída é o mesmo quórum que assina a
    /// distribuição.
    #[account(
        mut,
        token::mint = usdc_mint,
        token::authority = authority,
        token::token_program = usdc_token_program,
    )]
    pub origem_usdc: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = dom_mint,
        token::authority = vault.socios[0],
        token::token_program = dom_token_program,
    )]
    pub socio0_dom: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = dom_mint,
        token::authority = vault.socios[1],
        token::token_program = dom_token_program,
    )]
    pub socio1_dom: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = dom_mint,
        token::authority = vault.socios[2],
        token::token_program = dom_token_program,
    )]
    pub socio2_dom: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + FeeShareLedger::INIT_SPACE,
        seeds = [FEE_SHARE_SEED, vault.socios[0].as_ref()],
        bump,
    )]
    pub socio0_ledger: Box<Account<'info, FeeShareLedger>>,
    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + FeeShareLedger::INIT_SPACE,
        seeds = [FEE_SHARE_SEED, vault.socios[1].as_ref()],
        bump,
    )]
    pub socio1_ledger: Box<Account<'info, FeeShareLedger>>,
    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + FeeShareLedger::INIT_SPACE,
        seeds = [FEE_SHARE_SEED, vault.socios[2].as_ref()],
        bump,
    )]
    pub socio2_ledger: Box<Account<'info, FeeShareLedger>>,

    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
    pub usdc_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_deposit_especial(ctx: Context<DepositEspecial>, lucro_realizado: u64) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    require!(lucro_realizado > 0, DomError::ZeroLucro);

    // -----------------------------------------------------------------------
    // DV11 — a distribuição emite cota ao NAV. NAV velho emite cota errada, e
    // cota errada dilui quem já está dentro. Entra na trava como as outras.
    // -----------------------------------------------------------------------
    require_nav_fresco(
        ctx.accounts.vault.nav_ts,
        now,
        ctx.accounts.vault.max_nav_staleness,
    )?;

    let nav = ctx.accounts.vault.nav;
    let supply = ctx.accounts.dom_mint.supply;
    require!(supply > 0, DomError::EmptySupply);

    // -----------------------------------------------------------------------
    // DV10 — cadência. Quinzenal.
    //
    // Aqui a trava vem **antes** de qualquer movimento, ao contrário do
    // `accrue_performance`, onde ela ficava depois do desvio de "sem lucro".
    // Lá havia um caso inofensivo a proteger (chamada que não cobrava nada);
    // aqui não há: toda chamada move dinheiro, porque `P > 0` é pré-condição.
    //
    // `ultima_distribuicao_ts == 0` é "nunca distribuiu": a primeira não espera.
    // -----------------------------------------------------------------------
    if ctx.accounts.vault.ultima_distribuicao_ts > 0 {
        require!(
            now.saturating_sub(ctx.accounts.vault.ultima_distribuicao_ts)
                >= ctx.accounts.vault.distribuicao_interval,
            DomError::DistribuicaoMuitoCedo
        );
    }

    // -----------------------------------------------------------------------
    // Uma janela por vez — e esta distribuição **fecha** a anterior se ela
    // ficou aberta.
    //
    // O fecho normal é o `deploy_capital` do ciclo seguinte (D-F2-09): o
    // capital volta a campo, a janela acaba. Mas um ciclo pode não ter
    // re-deploy nenhum — capital que fica em casa é decisão legítima da mesa —
    // e aí a janela ficaria aberta para sempre, travando a distribuição
    // seguinte. Recusar aqui trocaria uma perda de direito por um impasse.
    //
    // Fechar aqui é seguro **porque a trava de cadência já passou**: quem tinha
    // direito teve a quinzena inteira para sacar. O que sobrou não some do
    // fundo — continua no preço da cota de quem não sacou.
    // -----------------------------------------------------------------------
    if ctx.accounts.vault.lucro_sacavel_restante > 0 {
        emit!(JanelaDeLucroEncerrada {
            nao_sacado: ctx.accounts.vault.lucro_sacavel_restante,
            timestamp: now,
        });
        ctx.accounts.vault.lucro_sacavel_restante = 0;
    }

    // -----------------------------------------------------------------------
    // A repartição — `D-F2-35`. **A ORDEM INVERTEU, e é o que faz 50 ser 50.**
    // -----------------------------------------------------------------------
    // Era: calcula a parte de CADA SÓCIO e multiplica por três. Com o campo por
    // sócio, `5000/3 = 1666,67` não é inteiro — 1666 dava 49,98% e 1667 daria
    // 50,01%. **Não existia valor que publicasse "50%" e cobrasse 50%.**
    //
    // Agora: calcula a parte DOS SÓCIOS de uma vez, a dos cotistas por
    // diferença, e só então divide por três. `P × 5000 / 10000` é exato, e a
    // parcela dos cotistas é o complemento exato.
    //
    // ⚠️ A SOBRA DA DIVISÃO POR TRÊS VAI PARA OS COTISTAS, e não para um sócio.
    //
    // São no máximo dois lamports por ciclo — e é justamente por ser pouco que
    // precisa de regra: sobra sem dono declarado é o tipo de coisa que alguém
    // "resolve" seis meses depois dando para quem estiver mais perto. Somada aos
    // cotistas, o arredondamento fica a favor do fundo, como na D9.
    // -----------------------------------------------------------------------
    require!(
        ctx.accounts.vault.perf_fee_bps_total >= MIN_PERF_FEE_BPS_TOTAL,
        DomError::ParametroForaDoLimite
    );

    let parcela_socios = u64::try_from(
        (lucro_realizado as u128)
            .checked_mul(ctx.accounts.vault.perf_fee_bps_total as u128)
            .ok_or(DomError::MathOverflow)?
            .checked_div(BPS_DEN)
            .ok_or(DomError::MathOverflow)?,
    )
    .map_err(|_| error!(DomError::MathOverflow))?;

    let parcela_por_socio = parcela_socios
        .checked_div(NUM_SOCIOS as u64)
        .ok_or(DomError::MathOverflow)?;

    // O que a divisão por três deixou para trás volta para os cotistas.
    let sobra = parcela_socios
        .checked_sub(
            parcela_por_socio
                .checked_mul(NUM_SOCIOS as u64)
                .ok_or(DomError::MathOverflow)?,
        )
        .ok_or(DomError::MathOverflow)?;

    let parcela_cotistas = lucro_realizado
        .checked_sub(parcela_socios)
        .ok_or(DomError::MathOverflow)?
        .checked_add(sobra)
        .ok_or(DomError::MathOverflow)?;

    // -----------------------------------------------------------------------
    // O NAV pós-distribuição.
    //
    //   patrimonio_antes = supply × nav
    //   nav_novo         = (patrimonio_antes + parcela_cotistas) / supply
    //   cotas_por_socio  = parcela_por_socio / nav_novo
    //
    // As cotas dos sócios saem ao NAV **já subido**, senão a conta não fecha:
    // emitir `60% de P` ao NAV antigo criaria cota de graça e abriria um buraco
    // de exatamente `60% de P` no patrimônio.
    //
    // Confere: (supply + 3 × cotas) × nav_novo <= patrimonio_antes + P, com a
    // diferença sendo só truncamento — a favor do cofre.
    // -----------------------------------------------------------------------
    let patrimonio_antes = usdc_from_shares(supply, nav)?;

    let nav_novo = u64::try_from(
        (patrimonio_antes as u128)
            .checked_add(parcela_cotistas as u128)
            .ok_or(DomError::MathOverflow)?
            .checked_mul(NAV_SCALE as u128)
            .ok_or(DomError::MathOverflow)?
            .checked_div(supply as u128)
            .ok_or(DomError::MathOverflow)?,
    )
    .map_err(|_| error!(DomError::MathOverflow))?;
    require!(nav_novo >= nav, DomError::InvalidNav);

    // O delta é o direito por cota na janela. Congelado agora, e não recalculado
    // no saque: o oráculo continua publicando durante a janela.
    let delta_lucro_por_cota = nav_novo - nav;

    // Truncado: cota de taxa a menos é lucro que fica com os cotistas (D9).
    let cotas_por_socio = shares_from_usdc(parcela_por_socio, nav_novo)?;

    // -----------------------------------------------------------------------
    // O dinheiro entra ANTES de qualquer emissão de cota.
    //
    // Ordem deliberada: se a transferência falhar — saldo insuficiente na
    // origem, conta congelada —, a transação inteira reverte e nenhuma cota de
    // sócio foi emitida contra lucro que não chegou.
    // -----------------------------------------------------------------------
    transfer_checked(
        CpiContext::new(
            ctx.accounts.usdc_token_program.key(),
            TransferChecked {
                from: ctx.accounts.origem_usdc.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.authority.to_account_info(),
            },
        ),
        lucro_realizado,
        ctx.accounts.usdc_mint.decimals,
    )?;

    let vault_bump = ctx.accounts.vault.bump;
    let vault_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, &[vault_bump]]];

    let destinos = [
        ctx.accounts.socio0_dom.to_account_info(),
        ctx.accounts.socio1_dom.to_account_info(),
        ctx.accounts.socio2_dom.to_account_info(),
    ];

    if cotas_por_socio > 0 {
        for destino in destinos.iter() {
            // -----------------------------------------------------------------
            // **Sem checagem de cap aqui** (D3.1/T14) — como no
            // `accrue_performance` que saiu. O cap de 25% protege a fila D+30 de
            // um cotista dominante; `fee_share` não usa a fila.
            // -----------------------------------------------------------------
            mint_to(
                CpiContext::new_with_signer(
                    ctx.accounts.dom_token_program.key(),
                    MintTo {
                        mint: ctx.accounts.dom_mint.to_account_info(),
                        to: destino.clone(),
                        authority: ctx.accounts.vault.to_account_info(),
                    },
                    vault_seeds,
                ),
                cotas_por_socio,
            )?;
        }
    }

    let bumps = [
        ctx.bumps.socio0_ledger,
        ctx.bumps.socio1_ledger,
        ctx.bumps.socio2_ledger,
    ];
    let socios = ctx.accounts.vault.socios;
    let livros = [
        &mut ctx.accounts.socio0_ledger,
        &mut ctx.accounts.socio1_ledger,
        &mut ctx.accounts.socio2_ledger,
    ];
    for (i, livro) in livros.into_iter().enumerate() {
        livro.socio = socios[i];
        livro.bump = bumps[i];
        livro.shares = livro
            .shares
            .checked_add(cotas_por_socio)
            .ok_or(DomError::MathOverflow)?;
    }

    let vault = &mut ctx.accounts.vault;
    vault.nav = nav_novo;
    vault.delta_lucro_por_cota = delta_lucro_por_cota;
    // Abre a janela. O teto é a parcela dos cotistas — nunca `supply × delta`,
    // que por truncamento pode ser um micro-USDC menor e deixaria o último a
    // sacar sem o dele.
    // -----------------------------------------------------------------------
    // ⚠️ O BOLO É O QUE OS COTISTAS CONSEGUEM SACAR, e não a parcela deles.
    // -----------------------------------------------------------------------
    // O direito individual é `cotas × delta`, e `delta = nav_novo − nav` já
    // passou por uma divisão truncada por `supply`. A soma de todos os direitos
    // é `supply × delta`, que pode ser ALGUNS LAMPORTS MENOR que a
    // `parcela_cotistas` que subiu o NAV.
    //
    // Gravar a parcela aqui deixava essa diferença **presa**: ninguém tem
    // direito a ela, o bolo nunca chega a zero, e `lucro_sacavel_restante > 0` é
    // o que mantém a JANELA ABERTA — que recusa aporte e transferência de cota.
    // Dois lamports de truncamento fechariam o fundo até a distribuição
    // seguinte.
    //
    // Descoberto pelo `D-F2-35`: com a repartição 60/40 as fixtures davam contas
    // redondas e a diferença era sempre zero. O defeito era latente desde o
    // início e nenhum ensaio o alcançava — só mudou o `P` e ele apareceu.
    //
    // A diferença fica na treasury, sem dono: o patrimônio passa a ser
    // levemente MAIOR que `supply × nav`, que é a direção segura.
    // -----------------------------------------------------------------------
    vault.lucro_sacavel_restante = usdc_from_shares(supply, delta_lucro_por_cota)?;
    vault.ultima_distribuicao_ts = now;

    emit!(LucroDistribuido {
        lucro_realizado,
        parcela_cotistas,
        parcela_socios,
        nav_antes: nav,
        nav_depois: nav_novo,
        delta_lucro_por_cota,
        cotas_por_socio,
        timestamp: now,
    });

    Ok(())
}
