use {
    crate::{
        constants::*,
        error::DomError,
        events::{ComissaoDeAfiliado, JanelaDeLucroEncerrada, LucroDistribuido},
        math::{require_nav_fresco, shares_from_usdc},
        state::{AfiliadoDoFechamento, FeeShareLedger, PosicaoDoCotista, Vault},
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
/// # Por onde o dinheiro chega (Upgrade J)
///
/// Da **gaveta** — a conta de USDC que a mesa apontou por `set_gaveta_usdc`
/// (em mainnet a ATA do vault 1 do Squads). Tudo que cai lá é lucro realizado
/// do ciclo: o `publish_nav` seguinte absorve no índice, e **não sai mais**.
/// No fechamento a dona da gaveta assina esta instrução e o P vai para o caixa
/// por CPI, aqui dentro — sem conta intermediária, numa proposta só.
///
/// O **capital** de volta continua entrando pelo `return_capital`, que não é
/// privilegiado. Aqui entra só o que passou do capital: o `P`.
///
/// # A janela de saque
///
/// Esta instrução **abre** a janela: grava `lucro_sacavel_restante = 40% de P`
/// e o `delta_lucro_por_cota`. Quem fecha é o `deploy_capital` do ciclo
/// seguinte — o marco é o re-deploy, não um cronômetro (D-F2-09).
/// # Contas restantes (`remaining_accounts`) — Upgrade J
///
/// `[0..3]` a `PosicaoDoCotista` de cada sócio, na ordem de `vault.socios`,
/// mutável. A cunhagem da mesa é uma ENTRADA: entra na média ponderada ao
/// índice de agora, senão as cotas novas reivindicariam o P do ciclo que
/// acabou de fechar. E o acerto pendente do sócio (se ele segurou cota o
/// ciclo inteiro) é compensado na própria cunhagem — crédito cunha a mais,
/// débito cunha a menos —, que é como a conta fecha sem a assinatura dele.
/// Vão em `remaining_accounts` porque três `Account` a mais na struct
/// estouravam a pilha do `try_accounts` (4.592 de 4.096 bytes). A conta tem
/// de existir (`abrir_posicao`, sem whitelist para sócio).
///
/// `[3..]` (J1) por linha da lista `afiliados`, na ordem dela, quatro contas:
/// `indicado_dom` (conta de cota do indicado — só leitura, dá as cotas),
/// `indicado_posicao` (a entrada dele), `afiliado_dom` (recebe a comissão,
/// mutável), `afiliado_posicao` (mutável: a comissão é uma entrada e o acerto
/// dele é compensado na cunhagem, como o dos sócios).
#[derive(Accounts)]
pub struct DepositEspecial<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// **A dona da gaveta** — em mainnet o vault 1 do Squads (`2nFNfsWi…`),
    /// assinando dentro da proposta 2/3 (`vaultIndex 1`). O privilégio vem de a
    /// mesa ter apontado a gaveta por `set_gaveta_usdc` (vault 0, 2/3): quem
    /// pode mover o P é quem fecha o ciclo, e o P sai da gaveta para o caixa
    /// por CPI aqui dentro — nenhuma conta intermediária, nenhuma proposta a
    /// mais. (D-F2-43 §6b.2, item 4 revertido em 21/09: gaveta com chave.)
    pub gaveta_owner: Signer<'info>,
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
    #[account(mut)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,
    /// A gaveta: a conta de USDC que a mesa apontou (`vault.gaveta_usdc`), cuja
    /// dona assina esta instrução. O P vem DIRETO dela — nunca passou por outra
    /// conta; nunca saiu do preço (J2).
    #[account(
        mut,
        constraint = gaveta.key() == vault.gaveta_usdc @ DomError::GavetaErrada,
        token::authority = gaveta_owner,
        token::token_program = usdc_token_program,
    )]
    pub gaveta: Box<InterfaceAccount<'info, TokenAccount>>,
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

pub fn handle_deposit_especial<'info>(
    ctx: Context<'info, DepositEspecial<'info>>,
    lucro_realizado: u64,
    afiliados: Vec<AfiliadoDoFechamento>,
) -> Result<()> {
    // -----------------------------------------------------------------------
    // Upgrade J — o fechamento pela D-F2-43 §4, na ordem obrigatoria:
    //
    //   1. le P da gaveta; confere p_ciclo == P (conservacao)
    //   2. mesa = P × taxa_mesa
    //   3. (J1) cunha os afiliados: comissao = ganho_indicado × taxa × bps
    //   4. cunha o resto da mesa para os socios; indice_diluicao anda
    //   5. move o P da gaveta para o treasury (a PDA assina)
    //   6. indice_ciclo_anterior/indice_ciclo/saldo visto/p_ciclo/piso/nav_fechamento; abre a janela
    //
    // O NAV NAO SOBE: o P ja' esta' no preco desde que chegou na gaveta. A unica
    // mudanca de preco e' a queda pela cunhagem da mesa — na proxima publicacao.
    // `lucro_realizado` e' o P que a PROPOSTA esperava: se a gaveta tiver outro
    // valor (alguem mandou entre a proposta e a execucao), recusa — a mesa vota
    // um numero, nao um "o que estiver la".
    // -----------------------------------------------------------------------
    let now = Clock::get()?.unix_timestamp;
    require_nav_fresco(
        ctx.accounts.vault.nav_ts,
        now,
        ctx.accounts.vault.max_nav_staleness,
    )?;
    let nav = ctx.accounts.vault.nav;
    let supply = ctx.accounts.dom_mint.supply;
    require!(supply > 0, DomError::EmptySupply);

    if ctx.accounts.vault.ultima_distribuicao_ts > 0 {
        require!(
            now.saturating_sub(ctx.accounts.vault.ultima_distribuicao_ts)
                >= ctx.accounts.vault.distribuicao_interval,
            DomError::DistribuicaoMuitoCedo
        );
    }
    if ctx.accounts.vault.lucro_sacavel_restante > 0 {
        emit!(JanelaDeLucroEncerrada {
            nao_sacado: ctx.accounts.vault.lucro_sacavel_restante,
            timestamp: now
        });
        ctx.accounts.vault.lucro_sacavel_restante = 0;
    }

    // 1 · o P, da gaveta, absorvido ate' o ultimo micro
    let gaveta_key = ctx.accounts.gaveta.key();
    let p = ctx.accounts.gaveta.amount;
    crate::indice::absorver_no_cofre(&mut ctx.accounts.vault, &gaveta_key, p, supply)?;
    require!(p == lucro_realizado, DomError::PDiferenteDoEsperado);
    require!(p > 0, DomError::ZeroLucro);
    require!(
        ctx.accounts.vault.p_ciclo == p,
        DomError::PConservacaoFalhou
    );

    // 2 · a mesa
    require!(
        ctx.accounts.vault.perf_fee_bps_total >= MIN_PERF_FEE_BPS_TOTAL,
        DomError::ParametroForaDoLimite
    );
    let parcela_socios = u64::try_from(
        (p as u128)
            .checked_mul(ctx.accounts.vault.perf_fee_bps_total as u128)
            .ok_or(DomError::MathOverflow)?
            .checked_div(BPS_DEN)
            .ok_or(DomError::MathOverflow)?,
    )
    .map_err(|_| error!(DomError::MathOverflow))?;
    // `parcela_socios` e' a MESA inteira (taxa × P); as comissoes dos afiliados
    // saem de dentro dela (J1) e o que sobra divide por tres — abaixo.

    // 4 · a mesa em cotas, ao NAV POS-diluicao: patrimonio − mesa, sobre o
    // supply de antes. Cunhar ao NAV publicado (com o P dentro) daria a mesa
    // cotas que, depois da propria cunhagem, valem menos que `mesa` — e o
    // excedente ficaria com os cotistas (~0,1% na simulacao de 18 carteiras).
    // E' o mesmo cuidado do contrato antigo ("as cotas dos socios saem ao NAV
    // ja' ajustado, senao a conta nao fecha"). Esse preco e' o nav_fechamento:
    // a janela e o acerto convertem por ele.
    let mesa_por_cota = (parcela_socios as u128)
        .checked_mul(NAV_SCALE as u128)
        .ok_or(DomError::MathOverflow)?
        .checked_div(supply as u128)
        .ok_or(DomError::MathOverflow)?;
    let nav_fechamento = u64::try_from(
        (nav as u128)
            .checked_sub(mesa_por_cota)
            .ok_or(DomError::MathOverflow)?,
    )
    .map_err(|_| error!(DomError::MathOverflow))?;
    require!(nav_fechamento > 0, DomError::InvalidNav);
    // 6a · o ciclo vira ANTES da cunhagem: o acerto dos sócios (abaixo) tem de
    // enxergar este fechamento — o ganho deles neste ciclo e a diluição desta
    // mesa —, e a entrada das cotas novas é ao índice de agora.
    {
        let vault = &mut ctx.accounts.vault;
        let passo = (parcela_socios as u128)
            .checked_mul(INDICE_SCALE)
            .ok_or(DomError::MathOverflow)?
            .checked_div(supply as u128)
            .ok_or(DomError::MathOverflow)?;
        vault.indice_diluicao = vault
            .indice_diluicao
            .checked_add(passo)
            .ok_or(DomError::MathOverflow)?;
        vault.indice_ciclo_anterior = vault.indice_ciclo;
        vault.indice_ciclo = vault.indice_p;
        vault.nav_fechamento = nav_fechamento;
    }

    let vault_bump = ctx.accounts.vault.bump;
    let vault_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, &[vault_bump]]];
    let socios = ctx.accounts.vault.socios;
    let taxa_bps = ctx.accounts.vault.perf_fee_bps_total;
    let mut cunhadas_total: u64 = 0;

    // 3 · (J1) os afiliados: comissao_i = ganho_indicado × taxa_mesa × bps.
    // O ganho do indicado e' o do ciclo que acabou de fechar — pelo indice,
    // com as cotas que ele tem agora (a mesma base do acerto dele). A comissao
    // sai da mesa: o cotista nao paga um micro a mais.
    require!(
        ctx.remaining_accounts.len() >= NUM_SOCIOS + afiliados.len() * CONTAS_POR_AFILIADO,
        DomError::AfiliadoInvalido
    );
    let mut comissoes_total: u64 = 0;
    for (n, linha) in afiliados.iter().enumerate() {
        require!(
            linha.bps >= 1 && linha.bps <= MAX_AFILIADO_BPS,
            DomError::AfiliadoInvalido
        );
        require!(linha.indicado != linha.afiliado, DomError::AfiliadoInvalido);
        require!(
            !afiliados[..n].iter().any(|a| a.indicado == linha.indicado),
            DomError::AfiliadoRepetido
        );
        let base = NUM_SOCIOS + n * CONTAS_POR_AFILIADO;
        let indicado_dom =
            InterfaceAccount::<TokenAccount>::try_from(&ctx.remaining_accounts[base])?;
        require!(
            indicado_dom.mint == ctx.accounts.dom_mint.key()
                && indicado_dom.owner == linha.indicado,
            DomError::AfiliadoInvalido
        );
        let indicado_posicao =
            carregar_posicao(&ctx.remaining_accounts[base + 1], &linha.indicado, false)?;
        let afiliado_dom =
            InterfaceAccount::<TokenAccount>::try_from(&ctx.remaining_accounts[base + 2])?;
        require!(
            afiliado_dom.mint == ctx.accounts.dom_mint.key()
                && afiliado_dom.owner == linha.afiliado
                && ctx.remaining_accounts[base + 2].is_writable,
            DomError::AfiliadoInvalido
        );
        let mut afiliado_posicao =
            carregar_posicao(&ctx.remaining_accounts[base + 3], &linha.afiliado, true)?;

        let v = &ctx.accounts.vault;
        let ganho_indicado = crate::indice::ganho(
            indicado_dom.amount,
            indicado_posicao.indice_entrada,
            v.indice_ciclo_anterior,
            v.indice_ciclo,
        )?;
        let comissao = u64::try_from(
            (ganho_indicado as u128)
                .checked_mul(taxa_bps as u128)
                .ok_or(DomError::MathOverflow)?
                .checked_mul(linha.bps as u128)
                .ok_or(DomError::MathOverflow)?
                / (BPS_DEN * BPS_DEN),
        )
        .map_err(|_| error!(DomError::MathOverflow))?;
        comissoes_total = comissoes_total
            .checked_add(comissao)
            .ok_or(DomError::MathOverflow)?;
        require!(comissoes_total <= parcela_socios, DomError::MathOverflow);

        let cotas = cunhar_com_acerto(
            v,
            &mut afiliado_posicao,
            &linha.afiliado,
            afiliado_dom.amount,
            &ctx.remaining_accounts[base + 2],
            shares_from_usdc(comissao, nav_fechamento)?,
            nav_fechamento,
            &ctx.accounts.dom_mint.to_account_info(),
            &ctx.accounts.dom_token_program,
            vault_seeds,
        )?;
        cunhadas_total = cunhadas_total
            .checked_add(cotas)
            .ok_or(DomError::MathOverflow)?;
        afiliado_posicao.exit(&crate::ID)?;
        emit!(ComissaoDeAfiliado {
            indicado: linha.indicado,
            afiliado: linha.afiliado,
            bps: linha.bps,
            ganho_indicado,
            comissao_usdc: comissao,
            cotas,
            timestamp: now,
        });
    }

    // 4 · o resto da mesa para os socios, com o acerto de cada um compensado
    // na cunhagem (J7 sem assinatura): cunha `cotas_por_socio + credito − debito`.
    // Se o debito passar disso — socio segurando mais de um terco do fundo —
    // ele assina `acertar` antes e o fechamento e' repetido.
    let mesa_dos_socios = parcela_socios - comissoes_total;
    let parcela_por_socio = mesa_dos_socios / NUM_SOCIOS as u64;
    let sobra = mesa_dos_socios - parcela_por_socio * (NUM_SOCIOS as u64); // fica com os cotistas (D9)
    let parcela_cotistas = p - parcela_socios + sobra;
    let cotas_por_socio = shares_from_usdc(parcela_por_socio, nav_fechamento)?;
    let contas = [
        ctx.accounts.socio0_dom.to_account_info(),
        ctx.accounts.socio1_dom.to_account_info(),
        ctx.accounts.socio2_dom.to_account_info(),
    ];
    let saldos = [
        ctx.accounts.socio0_dom.amount,
        ctx.accounts.socio1_dom.amount,
        ctx.accounts.socio2_dom.amount,
    ];
    for i in 0..NUM_SOCIOS {
        let mut posicao = carregar_posicao(&ctx.remaining_accounts[i], &socios[i], true)?;
        let cotas = cunhar_com_acerto(
            &ctx.accounts.vault,
            &mut posicao,
            &socios[i],
            saldos[i],
            &contas[i],
            cotas_por_socio,
            nav_fechamento,
            &ctx.accounts.dom_mint.to_account_info(),
            &ctx.accounts.dom_token_program,
            vault_seeds,
        )?;
        cunhadas_total = cunhadas_total
            .checked_add(cotas)
            .ok_or(DomError::MathOverflow)?;
        posicao.exit(&crate::ID)?;
    }
    let bumps = [
        ctx.bumps.socio0_ledger,
        ctx.bumps.socio1_ledger,
        ctx.bumps.socio2_ledger,
    ];
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

    // 5 · o P sai da gaveta para o caixa — a dona da gaveta assina (o vault 1,
    // por dentro da proposta)
    transfer_checked(
        CpiContext::new(
            ctx.accounts.usdc_token_program.key(),
            TransferChecked {
                from: ctx.accounts.gaveta.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.gaveta_owner.to_account_info(),
            },
        ),
        p,
        ctx.accounts.usdc_mint.decimals,
    )?;

    // 6 · o ciclo vira
    let vault = &mut ctx.accounts.vault;
    vault.gaveta_saldo_visto = 0;
    vault.p_ciclo = 0;
    vault.delta_lucro_por_cota = 0; // legado: a janela passa a ser pelo indice (J5)
    vault.lucro_sacavel_restante = parcela_cotistas;
    vault.ultima_distribuicao_ts = now;
    // o piso depois da cunhagem: (campo + caixa com o P) ÷ supply novo
    {
        let caixa = ctx
            .accounts
            .treasury
            .amount
            .checked_add(p)
            .ok_or(DomError::MathOverflow)?;
        let supply_novo = supply
            .checked_add(cunhadas_total)
            .ok_or(DomError::MathOverflow)?;
        let base = (vault.deployed_usdc as u128)
            .checked_add(caixa as u128)
            .ok_or(DomError::MathOverflow)?;
        vault.nav_piso = u64::try_from(
            base.checked_mul(NAV_SCALE as u128)
                .ok_or(DomError::MathOverflow)?
                / supply_novo as u128,
        )
        .map_err(|_| error!(DomError::MathOverflow))?;
    }

    emit!(LucroDistribuido {
        lucro_realizado: p,
        parcela_cotistas,
        parcela_socios,
        nav_antes: nav,
        nav_depois: nav,
        delta_lucro_por_cota: 0,
        cotas_por_socio,
        comissoes_afiliados: comissoes_total,
        timestamp: now,
    });

    Ok(())
}

/// Lê uma `PosicaoDoCotista` de `remaining_accounts`: PDA certa, dono certo,
/// gravável quando vai ser regravada.
fn carregar_posicao<'info>(
    info: &'info AccountInfo<'info>,
    owner: &Pubkey,
    gravavel: bool,
) -> Result<Account<'info, PosicaoDoCotista>> {
    let (esperada, _) = Pubkey::find_program_address(&[POSICAO_SEED, owner.as_ref()], &crate::ID);
    require_keys_eq!(info.key(), esperada, DomError::PosicaoDoCotistaAusente);
    require!(
        !gravavel || info.is_writable,
        DomError::PosicaoDoCotistaAusente
    );
    let posicao: Account<PosicaoDoCotista> = Account::try_from(info)?;
    require_keys_eq!(posicao.owner, *owner, DomError::PosicaoDoCotistaAusente);
    Ok(posicao)
}

/// Cunha `cotas_base` para uma carteira que NÃO assina o fechamento (sócio,
/// afiliado), compensando o acerto pendente dela na própria cunhagem: crédito
/// cunha a mais, débito cunha a menos; débito maior que a cunhagem é
/// `AcertoPendente` (o dono assina `acertar` antes). Registra a entrada ao
/// índice de agora e anda os odômetros. Devolve o que cunhou.
#[allow(clippy::too_many_arguments)]
fn cunhar_com_acerto<'info>(
    vault: &Account<'info, Vault>,
    posicao: &mut PosicaoDoCotista,
    owner: &Pubkey,
    cotas_antes: u64,
    conta_info: &AccountInfo<'info>,
    cotas_base: u64,
    nav_fechamento: u64,
    dom_mint: &AccountInfo<'info>,
    dom_token_program: &Interface<'info, TokenInterface>,
    vault_seeds: &[&[&[u8]]],
) -> Result<u64> {
    let mut cunhar = cotas_base;
    if !crate::indice::acertada(posicao, vault) {
        let a = crate::indice::acerto_de(cotas_antes, vault.perf_fee_bps_total, posicao, vault)?;
        if a.pago_micro > a.devido_micro {
            cunhar = cunhar
                .checked_add(shares_from_usdc(
                    a.pago_micro - a.devido_micro,
                    nav_fechamento,
                )?)
                .ok_or(DomError::MathOverflow)?;
        } else if a.devido_micro > a.pago_micro {
            let debito = shares_from_usdc(a.devido_micro - a.pago_micro, nav_fechamento)?;
            cunhar = cunhar.checked_sub(debito).ok_or(DomError::AcertoPendente)?;
        }
    }
    if cunhar > 0 {
        mint_to(
            CpiContext::new_with_signer(
                dom_token_program.key(),
                MintTo {
                    mint: dom_mint.clone(),
                    to: conta_info.clone(),
                    authority: vault.to_account_info(),
                },
                vault_seeds,
            ),
            cunhar,
        )?;
    }
    let bump = posicao.bump;
    crate::indice::registrar_entrada(posicao, owner, bump, cotas_antes, cunhar, vault.indice_p)?;
    posicao.indice_diluicao_visto = vault.indice_diluicao;
    posicao.indice_p_acertado = vault.indice_ciclo;
    Ok(cunhar)
}
