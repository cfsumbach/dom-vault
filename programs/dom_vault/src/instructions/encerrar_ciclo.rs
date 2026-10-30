//! # `encerrar_ciclo` — a saída que fecha um retrato com linha NÃO paga
//!
//! **A regra escrita** (DECISOES 13/10 16:55):
//!
//! > *"(3) marca de pago PDA `["pago", ciclo, indicado]`, fechável no encerramento
//! > para devolver o rent; (7) a sobra volta à mesa como cotas ao `nav_fechamento`
//! > DO RETRATO, automaticamente no lote que fizer `linhas_pagas =
//! > linhas_votadas`, **ou no `encerrar_ciclo`**; (8) `encerrar_ciclo` (mesa, 2 de
//! > 3) fecha o retrato com evento nomeando as linhas **não pagas**."*
//!
//! E a de 21:00 item 5: *"o `encerrar_ciclo` (2 de 3) é a saída que fecha um
//! retrato com linhas não pagas — teste obrigatório"*.
//!
//! ## ⚠️ A divergência que o devnet levantou, e como ela foi resolvida
//!
//! Um resumo de ordem dizia *"encerrar_ciclo só com retrato fechado e comissões
//! pagas"*, o que deixaria esta instrução sem função — com tudo pago, **o lote que
//! fecha já fecha** (`pagar_comissoes`, `fecha_o_ciclo`). O dono resolveu em
//! 14/10 23:00 item 3, textual: *"encerrar_ciclo pela **regra escrita** de 13/10
//! itens 7-8"*. Então é isto: o encerramento existe para o retrato que ficou
//! ABERTO com linha não paga, e é por isso que o `constraint` exige
//! `retrato.aberto`.
//!
//! ## Por que o evento é PROVA, e não declaração
//!
//! A lista de afiliados **não** é congelada na Fase A (decisão 9) — o que é
//! congelado é a CONTAGEM. Então o contrato não tem como saber *quais* linhas a
//! mesa votou. O que ele exige, e é o que torna o evento verificável:
//!
//! 1. `nao_pagas.len() + linhas_pagas == linhas_votadas` — a contagem **fecha**;
//! 2. cada pubkey declarada não paga **não tem marca** neste ciclo (a conta da PDA
//!    vem no `remaining_accounts` e tem de estar vazia);
//! 3. cada marca fechada **é** uma marca deste ciclo (a PDA é derivada do
//!    `indicado` que está DENTRO dela e conferida contra a chave da conta).
//!
//! ⚠️ **O limite, declarado:** isto prova *"esta carteira não foi paga neste
//! ciclo"* e *"as contas fecham"* — **não** prova que ela estava na lista votada,
//! porque a lista nunca foi gravada. Trocar uma não paga por outra carteira
//! qualquer sem marca produziria um evento com o nome errado e as contas certas.
//! Quem assina é a mesa, 2 de 3.

use {
    crate::{
        constants::*,
        error::DomError,
        events::CicloEncerrado,
        instructions::pagar_comissoes::devolver_sobra_a_mesa,
        state::{MarcaDePago, RetratoDoCiclo, Vault},
    },
    anchor_lang::{prelude::*, Discriminator},
    anchor_spl::token_interface::{Mint, TokenInterface},
};

#[derive(Accounts)]
#[instruction(ciclo: u64, nao_pagas: Vec<Pubkey>)]
pub struct EncerrarCiclo<'info> {
    /// Paga a taxa da transação.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// Para onde vai o rent das marcas fechadas: o destino nomeado pela mesa (registrado no evento).
    ///
    /// ⚠️ **O contrato não sabe quem pagou cada marca** — a `MarcaDePago` guarda
    /// `ciclo`, `indicado` e `bump`, não o pagador, e acrescentar campo nela
    /// quebraria as marcas que já existem. A regra (18/10 20:45 P3): o rent vai ao
    /// destino nomeado pela mesa (registrado no evento).
    ///
    /// 🔴 **E por que isto é conta NOMEADA e não o `payer`:** o devnet mediu que,
    /// com o rent indo ao payer da instrução, um encerramento de 32 marcas manda
    /// **~28,8 milhões de lamports para a autoridade** — que não foi quem pagou as
    /// marcas. A mesa nomeia o destino na proposta (2 de 3), como no 7.6, e o
    /// evento registra em `rent_para`. O mesmo desenho nas duas instruções.
    ///
    /// CHECK: não há tipo a conferir — é destino de lamports. O que protege é a
    /// assinatura 2 de 3, não o tipo da conta.
    #[account(mut)]
    pub destino_do_rent: UncheckedAccount<'info>,
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
    /// O retrato **aberto** do ciclo — é o que esta instrução existe para fechar.
    #[account(
        mut,
        seeds = [RETRATO_SEED, &ciclo.to_le_bytes()],
        bump = retrato.bump,
        constraint = retrato.aberto @ DomError::RetratoFechado,
        constraint = retrato.ciclo == ciclo @ DomError::RetratoDoCicloInvalido,
    )]
    pub retrato: Box<Account<'info, RetratoDoCiclo>>,
    #[account(mut)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

/// As contas que vêm no `remaining_accounts`, nesta ordem:
///
/// ```text
/// [0 .. 3)                      as 3 ATAs de cota dos sócios   (ordem de vault.socios)
/// [3 .. 6)                      as 3 posições dos sócios
/// [6 .. 6+n)                    as marcas das linhas NÃO PAGAS, conferidas VAZIAS
/// [6+n .. )                     as marcas de pago A FECHAR — OPCIONAIS (0, algumas ou todas)
/// ```
///
/// ⚠️ A ordem mudou depois de o devnet medir o teto: as não pagas vêm ANTES
/// porque são obrigatórias, e as marcas a fechar são o que sobra. Com 42+ linhas
/// votadas, exigir todas as marcas passava das 64 travas e o retrato não fechava.
pub fn handle_encerrar_ciclo<'info>(
    ctx: Context<'info, EncerrarCiclo<'info>>,
    ciclo: u64,
    nao_pagas: Vec<Pubkey>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let pagas = ctx.accounts.retrato.linhas_pagas as usize;
    let votadas = ctx.accounts.retrato.linhas_votadas as usize;

    // ═══════════════════════════════════════════════════════════════════════
    // 1 · A CONTAGEM FECHA — e é ela que faz o evento valer
    // ═══════════════════════════════════════════════════════════════════════
    require!(
        nao_pagas.len() + pagas == votadas,
        DomError::LinhasDoEncerramentoNaoFecham
    );
    // ⚠️ **AS MARCAS PAGAS A FECHAR SÃO OPCIONAIS, e isto é um conserto.**
    //
    // A primeira versão exigia `6 + linhas_votadas` contas — TODAS as marcas
    // pagas mais todas as não pagas. O devnet mediu o teto: com ~42 linhas
    // votadas a transação passa das **64 travas** do runtime e **o retrato não
    // fecha de jeito nenhum** — um retrato grande ficaria preso para sempre, que
    // é o oposto do que esta instrução existe para fazer.
    //
    // Agora o contrato exige só o que PROVA o encerramento: as 6 contas da mesa e
    // as marcas das NÃO PAGAS (conferidas vazias). O que vier depois é tratado
    // como marca a fechar — zero, algumas ou todas —, e o rent das que não vierem
    // continua recuperável num encerramento... que não existe mais. ⓘ Então fica
    // declarado: **marca não fechada é rent que fica na conta**, e isso é melhor
    // que um retrato que não fecha.
    let minimo = NUM_SOCIOS * 2 + nao_pagas.len();
    require!(
        ctx.remaining_accounts.len() >= minimo
            && ctx.remaining_accounts.len() <= NUM_SOCIOS * 2 + votadas,
        DomError::ContasDoLoteIncompletas
    );
    let marcas_a_fechar = ctx.remaining_accounts.len() - minimo;

    // ⚠️ Duas pubkeys iguais na lista de não pagas fechariam a contagem com uma
    // linha fantasma — e a segunda conferiria a MESMA PDA vazia.
    for (i, a) in nao_pagas.iter().enumerate() {
        for b in nao_pagas.iter().skip(i + 1) {
            require!(a != b, DomError::IndicadoRepetidoNoLote);
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 2 · AS LINHAS NÃO PAGAS: cada uma tem de estar SEM marca
    // ═══════════════════════════════════════════════════════════════════════
    for (i, indicado) in nao_pagas.iter().enumerate() {
        let info = &ctx.remaining_accounts[NUM_SOCIOS * 2 + i];
        let (esperada, _) = Pubkey::find_program_address(
            &[PAGO_SEED, &ciclo.to_le_bytes(), indicado.as_ref()],
            &crate::ID,
        );
        require_keys_eq!(info.key(), esperada, DomError::ContaDeMarcaDivergente);
        // Marca VAZIA = linha não paga. Dados existentes = a linha foi paga, e aí
        // declarar "não paga" é mentira e a instrução recusa.
        require!(
            info.data_is_empty(),
            DomError::LinhaDeclaradaNaoPagaEstaPaga
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 3 · AS MARCAS PAGAS: conferidas, fechadas, rent ao destino nomeado pela mesa
    // ═══════════════════════════════════════════════════════════════════════
    let mut rent_devolvido: u64 = 0;
    let mut fechadas: u16 = 0;
    for i in 0..marcas_a_fechar {
        let info = &ctx.remaining_accounts[minimo + i];
        require_keys_eq!(*info.owner, crate::ID, DomError::ContaDeMarcaDivergente);
        let marca: Account<MarcaDePago> = Account::try_from(info)?;
        require!(marca.ciclo == ciclo, DomError::ContaDeMarcaDivergente);
        // A PDA é derivada do `indicado` que está DENTRO da conta: conta forjada
        // com outro indicado não bate com a própria chave.
        let (esperada, _) = Pubkey::find_program_address(
            &[PAGO_SEED, &ciclo.to_le_bytes(), marca.indicado.as_ref()],
            &crate::ID,
        );
        require_keys_eq!(info.key(), esperada, DomError::ContaDeMarcaDivergente);

        // ─── o fechamento, à mão ───────────────────────────────────────────
        //
        // ⚠️ Não é `close =` do Anchor porque o NÚMERO de marcas varia (0..k) e
        // elas vêm no `remaining_accounts`. O efeito é o mesmo que o do Anchor:
        // lamports a zero (o runtime varre a conta no fim da transação) e o
        // discriminador de conta fechada gravado, para que nada reabra a mesma
        // conta DENTRO desta transação e leia dado velho.
        let rent = info.lamports();
        **info.try_borrow_mut_lamports()? -= rent;
        **ctx
            .accounts
            .destino_do_rent
            .to_account_info()
            .try_borrow_mut_lamports()? += rent;
        rent_devolvido = rent_devolvido
            .checked_add(rent)
            .ok_or(DomError::MathOverflow)?;
        let mut dados = info.try_borrow_mut_data()?;
        dados.fill(0);
        dados[..8].copy_from_slice(&CLOSED_ACCOUNT_DISCRIMINATOR);
        fechadas += 1;
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 4 · A SOBRA DO ORÇAMENTO VOLTA À MESA — pelo MESMO código do lote
    // ═══════════════════════════════════════════════════════════════════════
    let sobra = ctx
        .accounts
        .retrato
        .orcamento_micro
        .saturating_sub(ctx.accounts.retrato.gasto_micro);
    let nav_fechamento = ctx.accounts.retrato.nav_fechamento;
    let socios = ctx.accounts.vault.socios;
    let vault_bump = ctx.accounts.vault.bump;
    let vault_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, &[vault_bump]]];
    let (sobra_em_cotas, sobra_micro) = devolver_sobra_a_mesa(
        &ctx.accounts.vault,
        &socios,
        sobra,
        nav_fechamento,
        ctx.remaining_accounts,
        &ctx.accounts.dom_mint,
        &ctx.accounts.dom_token_program,
        vault_seeds,
    )?;

    // ═══════════════════════════════════════════════════════════════════════
    // 🔴 A JANELA NÃO É TOCADA AQUI — e a primeira versão tocava, duas vezes errada
    // ═══════════════════════════════════════════════════════════════════════
    //
    // A minha primeira versão somava `sobra_micro` a `lucro_sacavel_restante` com
    // o comentário *"como no lote que fecha"*. **As duas coisas eram falsas**, e o
    // devnet mediu as duas no `af8c9ad`:
    //
    //  1. **O lote que fecha NÃO soma a sobra.** Ele soma `cunhado_do_lote` — a
    //     COMISSÃO cunhada ao afiliado (`pagar_comissoes.rs`), que é outro
    //     dinheiro. Sem deploy no meio, o encerrar somava +1.512.498 e o lote
    //     somava +0 pelo mesmo fechamento: as DUAS saídas da sobra (16:55 item 7)
    //     divergiam na janela, o que a regra não permite — é o mesmo dinheiro.
    //  2. **Faltava a guarda `if lucro_sacavel_restante > 0`**, que o lote tem
    //     exactamente para isto: `deploy_capital` ZERA a janela (fecha o ciclo
    //     para saque), e somar a um zero REABRE o que a mesa fechou. Medido pelo
    //     devnet: Fase A → lote → `deploy_capital` de 1 micro (janela = 0) →
    //     encerrar → **janela = 1.512.498**. O encerramento reabria a janela.
    //
    // Então o encerramento **não escreve na janela**: ele não cunha comissão
    // nenhuma, e a sobra da mesa não entra pela regra do lote.
    //
    // ✅ DECIDIDO (DECISOES 18/10 20:45 P1): o resto fica no tesouro, de todos, e NÃO toca a janela.
    let _ = sobra_em_cotas;

    // ═══════════════════════════════════════════════════════════════════════
    // 5 · O RETRATO FECHA
    // ═══════════════════════════════════════════════════════════════════════
    //
    // ⓘ `gasto_micro` NÃO é reescrito: ele guarda o que foi pago em linhas, e é a
    // única prova durável do rateio (o mesmo cuidado que o lote que fecha toma).
    // Quem diz que não há mais dívida é o `aberto`.
    //
    // ⚠️ E o CONTADOR `reservas_em_aberto_micro` fica como está: encerrar não
    // paga nem liquida reserva nenhuma. Ele continua = a soma das reservas
    // abertas, e o ciclo seguinte o herda (dono, 14/10 16:00).
    let r = &mut ctx.accounts.retrato;
    r.aberto = false;

    emit!(CicloEncerrado {
        ciclo,
        linhas_votadas: votadas as u16,
        linhas_pagas: pagas as u16,
        nao_pagas,
        sobra_micro,
        sobra_em_cotas,
        gasto_micro: r.gasto_micro,
        orcamento_micro: r.orcamento_micro,
        reservas_em_aberto_micro: r.reservas_em_aberto_micro,
        marcas_fechadas: fechadas,
        rent_devolvido,
        rent_para: ctx.accounts.destino_do_rent.key(),
        timestamp: now,
        // `sobra_micro` = 3 × `por_socio` do `devolver_sobra_a_mesa` (pagar_comissoes.rs): a divisao e exata
        por_socio_micro: sobra_micro / NUM_SOCIOS as u64,
    });
    Ok(())
}

/// O discriminador que o Anchor grava numa conta fechada.
const CLOSED_ACCOUNT_DISCRIMINATOR: [u8; 8] = [255, 255, 255, 255, 255, 255, 255, 255];

#[allow(dead_code)]
fn _conferir_discriminador() {
    // ⓘ Trava de compilação: se o `MarcaDePago` mudar de discriminador, isto
    // continua compilando — o que importa aqui é que ele EXISTE e tem 8 bytes.
    const _: () = assert!(MarcaDePago::DISCRIMINATOR.len() == 8);
}
