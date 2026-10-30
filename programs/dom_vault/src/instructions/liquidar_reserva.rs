//! # 7.6 `liquidar_reserva` — a reserva do afiliado vira cota, ao NAV de agora
//!
//! **A regra escrita:**
//!
//! > **13/10 14:40:** *"a reserva é DÍVIDA, fora do piso e do patrimônio"*, e o
//! > rent da conta da reserva **vai ao destino nomeado pela mesa (registrado no evento)** (18/10 20:45 P3).
//! > **13/10 16:55 (9):** *"7.6 `liquidar_reserva` cunha ao `vault.nav` vigente e
//! > NÃO depende de retrato"*.
//! > **14/10 16:00:** *"o 7.6 decrementa o contador do retrato mais recente"*.
//!
//! ## As duas regras que parecem brigar, e não brigam
//!
//! "Não depende de retrato" (16:55) e "decrementa o contador do retrato mais
//! recente" (16:00) convivem assim: o retrato **vem**, porque é onde o contador
//! mora, mas ele **não precisa estar aberto** — liquidar uma reserva não tem nada
//! a ver com a Fase B de um ciclo. É essa a leitura implementada aqui, e o
//! `constraint` não exige `aberto`.
//!
//! ⚠️ **E ele tem de ser o MAIS RECENTE**, conferido por `e_o_corrente` contra o
//! cofre: decrementar um retrato velho deixaria o contador do atual alto, e é o
//! contador do atual que o backend usa para tirar a dívida do patrimônio antes de
//! publicar o NAV (`backend-nav/lib/passivo-do-cofre.ts`). Contador alto = NAV
//! baixo = cota vendida barata.
//!
//! ## Por que cunhar ao NAV vigente (e não ao `nav_fechamento`)
//!
//! A comissão do afiliado nasceu no fechamento, mas ficou em reserva porque não
//! alcançou o mínimo de 1 USDC. Entre o fechamento e a liquidação pode passar
//! ciclo. Cunhar ao preço do fechamento daria ao afiliado o ganho (ou a perda) do
//! período em que ele **não** tinha cota — às custas dos cotistas. A regra do dono
//! é o NAV vigente, e é o que está aqui.
//!
//! ⚠️ E o NAV vigente tem de estar FRESCO: `require_nav_fresco`, a mesma trava do
//! aporte. Cunhar ao NAV velho é o mesmo defeito do aporte ao NAV velho, com o
//! mesmo bolso pagando — o do cotista que já está dentro.

use {
    crate::{
        constants::*,
        error::DomError,
        events::ReservaLiquidada,
        math::{require_nav_fresco, shares_from_usdc},
        state::{PosicaoDoCotista, ReservaDeComissao, RetratoDoCiclo, Vault},
    },
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface},
};

#[derive(Accounts)]
#[instruction(ciclo_do_retrato: u64)]
pub struct LiquidarReserva<'info> {
    /// A mesa, 2 de 3 pelo Squads.
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
        has_one = dom_mint @ DomError::UnknownMint,
        constraint = !vault.paused @ DomError::Paused,
    )]
    pub vault: Box<Account<'info, Vault>>,
    /// O retrato MAIS RECENTE — é onde o contador mora. **Não precisa estar
    /// aberto** (16:55 (9)), mas precisa ser o atual (conferido no handler pelo
    /// `e_o_corrente`).
    #[account(
        mut,
        seeds = [RETRATO_SEED, &ciclo_do_retrato.to_le_bytes()],
        bump = retrato.bump,
        constraint = retrato.ciclo == ciclo_do_retrato @ DomError::RetratoDoCicloInvalido,
    )]
    pub retrato: Box<Account<'info, RetratoDoCiclo>>,
    /// A reserva do afiliado. Fecha aqui, e o rent vai para `destino_do_rent`.
    #[account(
        mut,
        seeds = [COMISSAO_SEED, reserva.owner.as_ref()],
        bump = reserva.bump,
        close = destino_do_rent,
    )]
    pub reserva: Box<Account<'info, ReservaDeComissao>>,
    /// A conta de cota do afiliado — **tem de ser a ATA dele**, conferida.
    #[account(mut)]
    pub afiliado_dom: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Para onde vai o rent da reserva: o destino nomeado pela mesa (registrado no evento).
    ///
    /// ⚠️ **O contrato não sabe quem pagou** — a `ReservaDeComissao` guarda
    /// `owner`, `acumulado_micro` e `bump`, e acrescentar um campo `pagador`
    /// quebraria as reservas que já existem na cadeia (a da devnet, ciclo 4, com
    /// 512.499 micro). A regra (18/10 20:45 P3): o rent vai ao destino nomeado
    /// pela mesa, 2 de 3, registrado no evento — a mesa pagou o rent, e isso basta.
    ///
    /// CHECK: não há tipo a conferir — é só um destino de lamports, nomeado pela
    /// mesa na proposta e publicado no evento `ReservaLiquidada.rent_para`. O que
    /// protege aqui é a assinatura 2 de 3, não o tipo da conta.
    #[account(mut)]
    pub destino_do_rent: UncheckedAccount<'info>,
    #[account(mut)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    /// Paga o rent da posição do afiliado, se ela ainda não existir.
    #[account(mut)]
    pub payer: Signer<'info>,
}

/// ⓘ A POSIÇÃO DO AFILIADO vem em `remaining_accounts[0]`, e não como conta
/// nomeada, pelo mesmo motivo da Fase B: `carregar_ou_criar_posicao` precisa de uma
/// referência com a vida da transação, e é isso que o `remaining_accounts` dá. A
/// conferência de quem ela é está dentro da função.
pub fn handle_liquidar_reserva<'info>(
    ctx: Context<'info, LiquidarReserva<'info>>,
    _ciclo_do_retrato: u64,
) -> Result<()> {
    require!(
        ctx.remaining_accounts.len() == 1,
        DomError::ContasDoLoteIncompletas
    );
    let now = Clock::get()?.unix_timestamp;

    // ═══════════════════════════════════════════════════════════════════════
    // 1 · O QUE HÁ PARA LIQUIDAR
    // ═══════════════════════════════════════════════════════════════════════
    let acumulado = ctx.accounts.reserva.acumulado_micro;
    // ⚠️ MEDIDO ANTES DO `close`: o Anchor move os lamports no fim da instrucao,
    // entao ler depois daria zero. A minha primeira versao gravava
    // `rent_devolvido: 0` FIXO no evento, e o devnet mediu 899.160 lamports se
    // movendo na corrida — evento que diz zero sobre dinheiro que andou e pior
    // que evento sem o campo.
    let rent_da_reserva = ctx.accounts.reserva.to_account_info().lamports();
    // ⚠️ Reserva vazia não se liquida: fecharia a conta do afiliado e queimaria o
    // rent sem lhe dar cota nenhuma.
    require!(acumulado > 0, DomError::ReservaVazia);
    let afiliado = ctx.accounts.reserva.owner;

    // A conta de cota é a ATA do afiliado — o mesmo cuidado da Fase B, onde
    // trocar a ORDEM das contas reatribuía dinheiro entre afiliados.
    require_keys_eq!(
        ctx.accounts.afiliado_dom.owner,
        afiliado,
        DomError::AfiliadoDaLinhaDivergente
    );
    require_keys_eq!(
        ctx.accounts.afiliado_dom.mint,
        ctx.accounts.dom_mint.key(),
        DomError::UnknownMint
    );
    crate::indice::exigir_ata(
        &ctx.accounts.afiliado_dom.key(),
        &afiliado,
        &ctx.accounts.dom_mint.key(),
        &ctx.accounts.dom_token_program.key(),
    )?;

    // ═══════════════════════════════════════════════════════════════════════
    // 2 · O RETRATO TEM DE SER O MAIS RECENTE
    // ═══════════════════════════════════════════════════════════════════════
    require!(
        ctx.accounts.retrato.e_o_corrente(&ctx.accounts.vault),
        DomError::RetratoNaoEOMaisRecente
    );

    // ═══════════════════════════════════════════════════════════════════════
    // 3 · AO NAV VIGENTE, E ELE TEM DE ESTAR FRESCO
    // ═══════════════════════════════════════════════════════════════════════
    let nav = ctx.accounts.vault.nav;
    require!(ctx.accounts.vault.nav_ts != 0, DomError::NavNeverPublished);
    require_nav_fresco(
        ctx.accounts.vault.nav_ts,
        now,
        ctx.accounts.vault.max_nav_staleness,
    )?;

    // ⌊acumulado × 1e6 / nav⌋ — truncado, a favor do cofre, mesmo abaixo de 1 USDC.
    let cotas = shares_from_usdc(acumulado, nav)?;

    // ═══════════════════════════════════════════════════════════════════════
    // 4 · A CUNHAGEM, pelo mesmo caminho da Fase A e da Fase B
    // ═══════════════════════════════════════════════════════════════════════
    let vault_bump = ctx.accounts.vault.bump;
    let vault_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, &[vault_bump]]];
    let cotas_antes = ctx.accounts.afiliado_dom.amount;
    let pos_info = &ctx.remaining_accounts[0];
    let dom_info = ctx.accounts.afiliado_dom.to_account_info();
    let (mut posicao, _criada) = crate::instructions::deposit_especial::carregar_ou_criar_posicao(
        pos_info,
        &afiliado,
        &ctx.accounts.vault,
        cotas_antes,
        &ctx.accounts.payer.to_account_info(),
        ctx.accounts.system_program.key(),
    )?;
    let cunhadas = crate::instructions::deposit_especial::cunhar_com_acerto(
        &ctx.accounts.vault,
        &mut posicao,
        &afiliado,
        cotas_antes,
        &dom_info,
        cotas,
        nav,
        &ctx.accounts.dom_mint.to_account_info(),
        &ctx.accounts.dom_token_program,
        vault_seeds,
    )?;
    posicao.exit(&crate::ID)?;

    // ═══════════════════════════════════════════════════════════════════════
    // 5 · O CONTADOR DO RETRATO MAIS RECENTE DECREMENTA
    // ═══════════════════════════════════════════════════════════════════════
    //
    // ⚠️ `saturating_sub` e não `checked_sub`: o contador é uma SOMA que atravessa
    // ciclos, e se ele estiver menor que esta reserva por um defeito antigo, a
    // alternativa seria travar a liquidação para sempre. Chegar a zero é o pior
    // caso, e zero é o lado do cofre (dívida menor nunca infla o NAV... — ⓘ na
    // verdade infla: dívida menor = patrimônio maior. Por isso o evento publica o
    // contador DEPOIS, e o ensaio do devnet confere contra a soma real das
    // reservas que sobram).
    let r = &mut ctx.accounts.retrato;
    r.reservas_em_aberto_micro = r.reservas_em_aberto_micro.saturating_sub(acumulado);
    let contador = r.reservas_em_aberto_micro;
    let ciclo_do_retrato = r.ciclo;

    // ⓘ A reserva é fechada pelo `close = destino_do_rent` do Anchor. O
    // `acumulado_micro` não é zerado à mão porque a conta deixa de existir.

    emit!(ReservaLiquidada {
        afiliado,
        acumulado_micro: acumulado,
        nav,
        cotas: cunhadas,
        reservas_em_aberto_micro: contador,
        ciclo_do_retrato,
        rent_devolvido: rent_da_reserva,
        rent_para: ctx.accounts.destino_do_rent.key(),
        timestamp: now,
    });
    Ok(())
}

/// ⓘ Nada da reserva vai para a mesa: não há cunhagem para sócio aqui, e o
/// `treasury` não é tocado. O ensaio do devnet confere exatamente isso (regra 12).
#[allow(dead_code)]
fn _nada_para_a_mesa() {}

#[allow(dead_code)]
fn _tipos_usados(_p: Option<PosicaoDoCotista>) {}
