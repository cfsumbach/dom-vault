use {
    crate::{
        constants::*,
        error::DomError,
        events::LucroSacadoPorCotista,
        math::{require_nav_fresco, shares_from_usdc, usdc_from_shares},
        state::{LucroSacado, PosicaoDoCotista, Vault},
    },
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{
        burn, transfer_checked, Burn, Mint, TokenAccount, TokenInterface, TransferChecked,
    },
};

/// Saque do lucro da janela. **Não é privilegiada** — quem assina é o cotista.
///
/// # O direito (Upgrade J, D-F2-43 §5)
///
/// O fechamento não sobe o NAV: o `P` já estava no preço da cota desde que
/// caiu na gaveta. O que o fechamento congela é o índice do ciclo
/// (`indice_ciclo`) e o `nav_fechamento`. O lucro de cada um é o ganho DELE
/// pelo índice, menos a taxa da mesa:
///
/// ```text
///   ganho_i   = cotas_i × (indice_ciclo − max(indice_entrada_i, indice_ciclo_anterior)) ÷ 1e18
///   cotista_i = ganho_i × (1 − taxa_mesa)
/// ```
///
/// Quem entrou no meio do ciclo tem `indice_entrada` maior e leva só o `P`
/// que caiu depois dele — a régua "por cota" que pagou Filipe com o dinheiro
/// de Egnon (20/09) morreu aqui. O `delta_lucro_por_cota` fica no layout, a
/// zero, só pela compatibilidade de offsets.
///
/// # Por que queima cota
///
/// Pagar `cotas × delta` sem queimar tiraria USDC da treasury deixando o supply
/// intacto — o NAV teria que cair, e a queda diluiria justamente quem **não**
/// sacou. Queimando `valor / nav` cotas, patrimônio e supply caem juntos e o
/// NAV não se mexe.
///
/// O efeito para quem saca é o certo: sai com o lucro em USDC e volta à posição
/// em valor que tinha **antes** da distribuição. Confere —
/// `(C - valor/nav) × nav = C×nav - C×delta = C × (nav - delta)`, que é o
/// patrimônio dele ao NAV pré-distribuição.
///
/// # As três travas
///
/// 1. **marca por carteira** (`LucroSacado`) — pagar não zera `cotas`, então
///    sem marca a mesma carteira saca em laço: 196,36 · 189,47 · 182,83 · …
/// 2. **teto global** (`lucro_sacavel_restante`) — segunda cinta contra o bolo
///    pagar mais do que recebeu.
/// 3. **entrada fechada na janela** — `deposit` recusa e o hook barra
///    transferência de cota enquanto a janela está aberta. Sem isso, quem entra
///    depois da distribuição leva lucro que não gerou, e quem já sacou passa as
///    cotas para uma carteira virgem e saca de novo.
///
/// # Sócio não entra
///
/// Decisão da mesa (D-F2-09): sócio saca pela porta dele, o `redeem_fee_share`,
/// que é instantâneo e não depende de janela. Deixá-lo aqui também abriria a
/// única brecha aritmética que sobrava — as cotas que ele acabou de receber
/// existem no instante do saque e reivindicariam parte do bolo dos cotistas,
/// que é dinheiro que não é dele.
#[derive(Accounts)]
pub struct SacarLucro<'info> {
    #[account(mut)]
    pub cotista: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = dom_mint @ DomError::UnknownMint,
        has_one = usdc_mint @ DomError::UnknownUsdcMint,
        has_one = treasury @ DomError::InvalidTreasury,
        constraint = !vault.paused @ DomError::Paused,
    )]
    pub vault: Box<Account<'info, Vault>>,
    /// Marca de "já sacou esta distribuição". Criada no primeiro saque desta
    /// carteira — quem nunca saca não paga rent nenhum.
    #[account(
        init_if_needed,
        payer = cotista,
        space = 8 + LucroSacado::INIT_SPACE,
        seeds = [LUCRO_SEED, cotista.key().as_ref()],
        bump,
    )]
    pub marca: Box<Account<'info, LucroSacado>>,

    /// Upgrade J (J5/J7): a posição do cotista no índice — o direito da janela
    /// sai dela, e o acerto da taxa roda antes (o cotista assina: o débito passa).
    #[account(
        mut,
        seeds = [POSICAO_SEED, cotista.key().as_ref()],
        bump = posicao.bump,
        constraint = posicao.owner == cotista.key() @ DomError::PosicaoDoCotistaAusente,
    )]
    pub posicao: Box<Account<'info, PosicaoDoCotista>>,
    #[account(mut)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        mut,
        token::mint = dom_mint,
        token::authority = cotista,
        token::token_program = dom_token_program,
    )]
    pub cotista_dom: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = usdc_mint,
        token::authority = cotista,
        token::token_program = usdc_token_program,
    )]
    pub cotista_usdc: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
    pub usdc_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_sacar_lucro(ctx: Context<SacarLucro>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let cotista = ctx.accounts.cotista.key();

    // TRAVA 1 — a janela existe? `restante > 0` **é** a janela aberta.
    require!(
        ctx.accounts.vault.lucro_sacavel_restante > 0,
        DomError::JanelaDeLucroFechada
    );

    // Sócio tem porta própria (D-F2-09).
    require!(
        !ctx.accounts.vault.socios.contains(&cotista),
        DomError::SocioForaDaJanela
    );

    // TRAVA 2 — uma vez por distribuição. O carimbo é estritamente crescente,
    // então ele sozinho identifica a janela; não há contador de ciclo a manter.
    require!(
        ctx.accounts.marca.ultima_distribuicao_sacada < ctx.accounts.vault.ultima_distribuicao_ts,
        DomError::LucroJaSacado
    );

    // DV11 — a queima converte cota em USDC ao NAV corrente. É leitura de NAV
    // que vira dinheiro: entra na trava de frescura.
    require_nav_fresco(
        ctx.accounts.vault.nav_ts,
        now,
        ctx.accounts.vault.max_nav_staleness,
    )?;

    // -----------------------------------------------------------------------
    // Upgrade J (D-F2-43 §5): a parte do cotista na janela é a parcela dos
    // cotistas sobre o ganho DELE no ciclo que fechou — pelo índice, não por
    // cota:
    //
    //   cotista_i = cotas × (indice_ciclo − max(entrada, indice_ciclo_anterior))
    //               × (1 − taxa_mesa)
    //
    // e a queima é ao `nav_fechamento` gravado no cofre, não ao publicado de
    // hoje — o oráculo publica durante a janela e a ordem de chegada não pode
    // mudar quantas cotas cada um entrega. Antes de tudo, o acerto da taxa
    // (J7): o cotista assina, então crédito e débito passam; sem ele o saldo
    // de cotas que entra na conta seria o de antes do fechamento.
    // -----------------------------------------------------------------------
    crate::indice::acertar_com_cpi(
        &mut ctx.accounts.vault,
        &mut ctx.accounts.posicao,
        &ctx.accounts.cotista_dom,
        &ctx.accounts.dom_mint,
        &ctx.accounts.cotista.to_account_info(),
        true,
        &ctx.accounts.dom_token_program,
    )?;
    ctx.accounts.cotista_dom.reload()?;
    ctx.accounts.dom_mint.reload()?;

    let nav = ctx.accounts.vault.nav_fechamento;
    require!(nav > 0, DomError::JanelaDeLucroFechada);
    let cotas = ctx.accounts.cotista_dom.amount;
    require!(cotas > 0, DomError::SemLucroASacar);

    let ganho_no_ciclo = crate::indice::ganho(
        cotas,
        ctx.accounts.posicao.indice_entrada,
        ctx.accounts.vault.indice_ciclo_anterior,
        ctx.accounts.vault.indice_ciclo,
    )?;
    let parcela_cotistas_bps = BPS_DEN
        .checked_sub(ctx.accounts.vault.perf_fee_bps_total as u128)
        .ok_or(DomError::MathOverflow)?;
    let bruto = u64::try_from(
        (ganho_no_ciclo as u128)
            .checked_mul(parcela_cotistas_bps)
            .ok_or(DomError::MathOverflow)?
            / BPS_DEN,
    )
    .map_err(|_| error!(DomError::MathOverflow))?;

    let valor = bruto.min(ctx.accounts.vault.lucro_sacavel_restante);
    require!(valor > 0, DomError::SemLucroASacar);

    // -----------------------------------------------------------------------
    // Piso do saque, em USDC (Upgrade D). **Depois do teto do bolo, de
    // proposito:** a regra e' sobre o que o saque PAGA, e o que paga e' o
    // `valor` ja' limitado — nao o bruto. Travar no bruto deixaria passar um
    // saque que, aparado pelo bolo, pagaria menos que o piso.
    //
    // A unidade e' USDC porque a regua e' dinheiro. No modelo de NAV a cota nao
    // se multiplica com o lucro: o cotista termina o ciclo com as mesmas cotas,
    // cada uma valendo mais. Piso em cotas mediria o tamanho da posicao, nao o
    // tamanho do saque.
    //
    // Quem fica abaixo nao perde nada — o lucro segue no preco da cota e volta
    // na proxima distribuicao. E' adiamento, nao confisco.
    // -----------------------------------------------------------------------
    require!(
        valor >= ctx.accounts.vault.min_saque_lucro_usdc,
        DomError::SaqueLucroAbaixoDoMinimo
    );

    // -----------------------------------------------------------------------
    // Piso de caixa: reserva **mais** o passivo congelado da fila — o mesmo
    // piso do `deploy_capital`, e pela mesma razão. **A fila tem prioridade
    // sobre o saque de lucro.**
    //
    // No fluxo normal esta trava não morde: a janela abre logo depois do
    // `deposit_especial`, com o capital em casa e o `P` recém-creditado. Se
    // morder, é porque o cofre está apertado — e aí pagar lucro na frente da
    // fila seria exatamente o erro.
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
    // Mesmo piso do `deploy_capital`: a reserva, e só. O passivo congelado saiu
    // com a fila D+30 — o resgate de capital não se paga do treasury.
    let piso = reserva;
    let livre = ctx.accounts.treasury.amount.saturating_sub(piso);
    require!(valor <= livre, DomError::NoFreeCash);

    // Cotas a queimar, ao NAV corrente. Truncado a favor do cofre.
    let cotas_queimadas = shares_from_usdc(valor, nav)?;
    require!(cotas_queimadas > 0, DomError::SemLucroASacar);
    // Só acontece se o NAV desabar abaixo do delta entre a distribuição e o
    // saque. Recusa em vez de queimar mais do que a carteira tem.
    require!(cotas_queimadas <= cotas, DomError::MathOverflow);

    // Queima primeiro, paga depois. Queima não passa pelo hook (D17) — e é bom
    // que não passe: a janela aberta barra transferência, e o hook não sabe
    // distinguir uma queima legítima de uma fuga de cota.
    burn(
        CpiContext::new(
            ctx.accounts.dom_token_program.key(),
            Burn {
                mint: ctx.accounts.dom_mint.to_account_info(),
                from: ctx.accounts.cotista_dom.to_account_info(),
                authority: ctx.accounts.cotista.to_account_info(),
            },
        ),
        cotas_queimadas,
    )?;

    let vault_bump = ctx.accounts.vault.bump;
    let vault_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, &[vault_bump]]];
    let decimais = ctx.accounts.usdc_mint.decimals;
    transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.usdc_token_program.key(),
            TransferChecked {
                from: ctx.accounts.treasury.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.cotista_usdc.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            vault_seeds,
        ),
        valor,
        decimais,
    )?;

    let marca_bump = ctx.bumps.marca;
    let carimbo = ctx.accounts.vault.ultima_distribuicao_ts;
    ctx.accounts.marca.owner = cotista;
    ctx.accounts.marca.bump = marca_bump;
    ctx.accounts.marca.ultima_distribuicao_sacada = carimbo;

    ctx.accounts.vault.lucro_sacavel_restante -= valor;

    emit!(LucroSacadoPorCotista {
        cotista,
        cotas_queimadas,
        usdc: valor,
        nav,
        restante: ctx.accounts.vault.lucro_sacavel_restante,
    });

    Ok(())
}
