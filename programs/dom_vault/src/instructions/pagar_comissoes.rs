//! # FASE B DO FECHAMENTO EM LOTES — `pagar_comissoes`
//!
//! Aprovada pelo dono em 13/10 16:55, com dez decisões. A Fase A congelou o ciclo
//! num `RetratoDoCiclo`; esta instrução paga as comissões **em lotes de até 8**,
//! contra aquele retrato.
//!
//! ─── por que ela existe ─────────────────────────────────────────────────────
//! Medição do `cc-fundo-devnet` (13/10): com a lista real de produção o
//! `deposit_especial` pede **75 contas distintas contra o teto de 64** da rede, e
//! sem tabela de endereços cabem **2 linhas** de afiliado. O fechamento em uma
//! transação **não cabe**. Em lotes, cada transação é pequena e o número de linhas
//! deixa de importar.
//!
//! ─── ⚠️ DE ONDE VEM O VALOR DE CADA LINHA ───────────────────────────────────
//! Do **voto da mesa**, por linha, e não de um recálculo aqui.
//!
//! A razão é uma ordem do dono (13/10 19:40, textual): *"o direito do afiliado é
//! FIXADO NO FECHAMENTO, e nada do que o indicado fizer depois da Fase A reduz a
//! comissão"*. Recalcular aqui leria `indicado_dom.amount` **na hora do lote** —
//! e entre a Fase A e o lote o indicado pode sacar (o `sacar_lucro` **queima**
//! cota), resgatar ou **transferir** as cotas, encolhendo a comissão de quem o
//! indicou. A direção do erro é sempre contra o afiliado.
//!
//! O mecanismo que o dono **prefere** é outro — gravação preguiçosa das cotas do
//! fechamento na posição do indicado —, e ele **exige realloc** das 42 posições da
//! mainnet (89 bytes, folga zero → 97). A ordem é *"realloc da posição ou estouro
//! do teto = PARE e traga os números"*, e os números estão em
//! `dom-arquitetura · PEDIDO-FUNDO-PARA-ARQUITETO-GANHO-DO-INDICADO-13-10.md`
//! (336 bytes, 0,00233856 SOL). Enquanto a escolha não vem, vale o voto — que
//! cumpre a regra e **não** para no realloc.
//!
//! ⚠️ **O que o contrato deixa de conferir, dito sem adoçar:** ele não verifica
//! que a comissão corresponde ao ganho do indicado. Ele confere o que ainda dá:
//! o **teto do orçamento** votado na Fase A, a **unicidade** pelo par
//! `(ciclo, indicado)`, a **contagem** de linhas, e que o orçamento já saiu da
//! parte da mesa. O cotista não paga um micro em nenhum caminho.
//!
//! ─── o que NÃO muda em relação ao fechamento de hoje ────────────────────────
//! O mínimo de 1 USDC, a reserva própria do afiliado, o `exigir_ata` nas contas de
//! cota, o arredondamento a favor do cofre e o acerto compensado na cunhagem: tudo
//! vem das **mesmas funções** que o `deposit_especial` usa. Duas fontes de verdade
//! sobre dinheiro não se alinham — uma delas para de calcular.
use {
    crate::{
        constants::*,
        error::DomError,
        events::{
            ComissaoAcumulada, ComissaoDaReservaPaga, ComissaoDeAfiliado, LoteDeComissaoPago,
        },
        state::{LinhaDoLote, PosicaoDoCotista, ReservaDeComissao, RetratoDoCiclo, Vault},
    },
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface},
};

/// Quantas linhas cabem num lote. **Decisão 3 do dono (16:55).**
///
/// ⓘ O número saiu do teto da rede: `7 fixas + 6 por linha + 6 do embrulho do
/// Squads ≤ 64` dá `k ≤ 8`. Com o caminho do voto são **4** contas por linha, e o
/// mesmo k=8 fica com 19 travas de folga — ver a tabela em
/// `tests/fase_b_teto_da_transacao.rs`.
pub const K_MAX_DO_LOTE: usize = 8;

/// Quantas contas cada linha do lote traz.
///
/// `afiliado_dom`, `afiliado_posicao`, `afiliado_reserva`, `marca_pago`.
pub const CONTAS_POR_LINHA_DO_LOTE: usize = 4;

/// Quantas contas o lote que ENCERRA traz a mais: as 3 ATAs dos sócios e as 3
/// posições deles, para a sobra voltar à mesa em cotas (**decisão 6**).
pub const CONTAS_DA_SOBRA: usize = 6;

#[derive(Accounts)]
#[instruction(ciclo: u64)]
pub struct PagarComissoes<'info> {
    /// Paga a taxa e o rent das contas que nascem (a marca de pago, a posição do
    /// afiliado que ainda não tem, a reserva).
    #[account(mut)]
    pub payer: Signer<'info>,
    /// A mesa, 2 de 3 pelo Squads. ⚠️ **Sem ela, qualquer carteira poderia pagar
    /// uma linha inventada** — a lista não é congelada na Fase A (decisão 9), e é
    /// o voto que a autoriza.
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
    /// O retrato do ciclo — **aberto**, e é dele que saem o `nav_fechamento`, o
    /// orçamento e as contagens.
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

pub fn handle_pagar_comissoes<'info>(
    ctx: Context<'info, PagarComissoes<'info>>,
    ciclo: u64,
    linhas: Vec<LinhaDoLote>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    // ═══════════════════════════════════════════════════════════════════════
    // 1 · O LOTE: tamanho, contas e a contagem contra o retrato
    // ═══════════════════════════════════════════════════════════════════════
    require!(!linhas.is_empty(), DomError::LoteVazio);
    require!(linhas.len() <= K_MAX_DO_LOTE, DomError::LoteAcimaDoK);

    let retrato = &ctx.accounts.retrato;
    let fecha_o_ciclo =
        (retrato.linhas_pagas as usize) + linhas.len() == retrato.linhas_votadas as usize;

    // ⚠️ A CONTAGEM NUNCA PASSA DO VOTADO. Sem isto, um lote a mais pagaria linhas
    // que a mesa não votou — e a decisão 9 congelou a CONTAGEM justamente para que
    // ela fosse a trava, já que a lista não é congelada.
    require!(
        (retrato.linhas_pagas as usize) + linhas.len() <= retrato.linhas_votadas as usize,
        DomError::LinhasAcimaDoVotado
    );

    let esperadas =
        linhas.len() * CONTAS_POR_LINHA_DO_LOTE + if fecha_o_ciclo { CONTAS_DA_SOBRA } else { 0 };
    // ⚠️ IGUALDADE, não `>=`: conta sobrando é conta que ninguém conferiu, e a
    // lição do item 7 é que o cliente que monta a proposta erra a contagem — então
    // o contrato recusa alto em vez de ler a conta da linha seguinte no lugar da
    // reserva.
    require!(
        ctx.remaining_accounts.len() == esperadas,
        DomError::ContasDoLoteIncompletas
    );

    // ═══════════════════════════════════════════════════════════════════════
    // 2 · ⚠️ AFILIADO REPETIDO NO MESMO LOTE: RECUSA
    // ═══════════════════════════════════════════════════════════════════════
    // Duas linhas do mesmo afiliado trariam a MESMA conta de posição duas vezes.
    // Cada `Account::try_from` desserializa por conta própria, então a segunda
    // gravação sobrescreveria a primeira — o defeito que custou +24.418 micro de
    // ganho e +1.220 de comissão no ensaio de 13/10.
    //
    // ⓘ **E o dinheiro é o mesmo:** a reserva do afiliado acumula ENTRE lotes, e o
    // mínimo de 1 USDC vale para a soma dela. Pôr cada linha do mesmo afiliado num
    // lote diferente dá o mesmo total ao micro. O que a recusa custa é um lote a
    // mais, não dinheiro.
    // O indicado repetido é pego aqui, cedo e barato — duas linhas do mesmo
    // indicado tentariam criar a MESMA marca de pago, e a segunda falharia com o
    // erro opaco do System Program em vez de dizer o que aconteceu.
    for (i, a) in linhas.iter().enumerate() {
        for b in linhas.iter().skip(i + 1) {
            require!(a.indicado != b.indicado, DomError::IndicadoRepetidoNoLote);
        }
    }
    // ⚠️ O AFILIADO repetido só se conhece DENTRO do laço: a pubkey dele sai da
    // CONTA (`afiliado_dom.owner`), não do argumento — é o idioma do Upgrade M. A
    // conferência está no laço, contra os já vistos.

    let nav_fechamento = retrato.nav_fechamento;
    let orcamento = retrato.orcamento_micro;
    let gasto_antes = retrato.gasto_micro;
    let vault_bump = ctx.accounts.vault.bump;
    let vault_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, &[vault_bump]]];

    let mut gasto_do_lote: u64 = 0;
    let mut cunhado_do_lote: u64 = 0;
    // ⚠️ DECISOES 13/10 21:00 · a RESERVA é DÍVIDA e sai do piso, e o total vive no
    // retrato. Se a Fase B mexer nas reservas e NÃO atualizar o contador, o
    // fechamento seguinte desconta a dívida errada — o defeito que o devnet mediu
    // voltaria pela porta de trás.
    let mut reservas_guardadas: u64 = 0;
    let mut reservas_pagas: u64 = 0;
    // Os afiliados ja pagos neste lote. ⚠️ K_MAX_DO_LOTE e 8, entao a varredura
    // quadratica custa no maximo 28 comparacoes de pubkey — e o alternativo
    // (re-desserializar contas) foi o que levou o laco do M a O(n2) de CU.
    let mut afiliados_vistos: Vec<Pubkey> = Vec::with_capacity(linhas.len());

    // ═══════════════════════════════════════════════════════════════════════
    // 3 · AS LINHAS
    // ═══════════════════════════════════════════════════════════════════════
    for (n, linha) in linhas.iter().enumerate() {
        let base = n * CONTAS_POR_LINHA_DO_LOTE;
        let afiliado_dom_info = &ctx.remaining_accounts[base];
        let afiliado_posicao_info = &ctx.remaining_accounts[base + 1];
        let reserva_info = &ctx.remaining_accounts[base + 2];
        let marca_info = &ctx.remaining_accounts[base + 3];

        // ─── a conta de cota do afiliado, e de onde sai a pubkey dele ───────
        //
        // ⚠️ `exigir_ata`: a conta tem de ser a ATA canônica da carteira. Sem isso,
        // uma conta de token qualquer com o `owner` certo receberia a cunhagem — e
        // o confisco medido no ensaio de 13/10 foi de ~240 USDC.
        let afiliado_dom: InterfaceAccount<TokenAccount> =
            InterfaceAccount::try_from(afiliado_dom_info)?;
        require_keys_eq!(
            afiliado_dom.mint,
            ctx.accounts.dom_mint.key(),
            DomError::UnknownMint
        );
        let afiliado = afiliado_dom.owner;
        // ⚠️ O VALOR DA LINHA É AMARRADO AO AFILIADO VOTADO.
        //
        // A primeira versão não amarrava: o `micro` vinha no argumento e quem o
        // recebia era `remaining_accounts[n*4].owner`. Trocar a ORDEM das contas,
        // com os mesmos dados da instrução, reatribuía o dinheiro — medido pela
        // revisão adversária: `[{i0, 200 USDC}, {i1, 1 USDC}]` com as contas
        // trocadas pagou 190.476.190 cotas ao afiliado errado.
        //
        // Não era furo de autorização (a vault transaction do Squads guarda as
        // chaves E a ordem, então o 2-de-3 aprova a ordem), era furo de
        // CONFERÊNCIA: o contrato aceitava a proposta trocada, e os revisores
        // teriam de casar 32 bytes crus com a lista votada a olho.
        require_keys_eq!(
            afiliado,
            linha.afiliado,
            DomError::AfiliadoDaLinhaDivergente
        );
        // ⚠️ DUAS LINHAS DO MESMO AFILIADO NO MESMO LOTE: RECUSA.
        //
        // A conta de posição dele viria duas vezes na lista, e cada
        // `Account::try_from` desserializa por conta própria — a segunda gravação
        // sobrescreveria a primeira. É o defeito que custou +24.418 micro de ganho
        // e +1.220 de comissão no ensaio de 13/10, e aqui ele apareceria partido
        // em pedaços.
        //
        // ⓘ E o dinheiro é o mesmo: a reserva do afiliado acumula ENTRE lotes, e o
        // mínimo de 1 USDC vale para a soma dela. Pôr cada linha do mesmo afiliado
        // num lote diferente dá o mesmo total ao micro — o que a recusa custa é um
        // lote a mais, não dinheiro.
        require!(
            !afiliados_vistos.contains(&afiliado),
            DomError::AfiliadoRepetido
        );
        afiliados_vistos.push(afiliado);
        crate::indice::exigir_ata(
            &afiliado_dom_info.key(),
            &afiliado,
            &ctx.accounts.dom_mint.key(),
            &ctx.accounts.dom_token_program.key(),
        )?;
        // ⚠️ `AfiliadoInvalido` e não `ContaDeCotaNaoEAta`: conta não gravável não é
        // "não é ATA", e os dois caminhos ficariam indistinguíveis no log. É o mesmo
        // erro que o `deposit_especial` usa para esta condição.
        require!(afiliado_dom_info.is_writable, DomError::AfiliadoInvalido);
        // ⚠️ E o afiliado NÃO pode ser o próprio indicado: é a recusa que o painel
        // já conhece (`PEDIDO-FUNDO-PARA-PAINEL-SELOS-QUE-RECUSAM-13-10.md`).
        require_keys_neq!(afiliado, linha.indicado, DomError::AfiliadoIgualIndicado);

        // ─── a marca de pago: `["pago", ciclo, indicado]` ───────────────────
        //
        // ⚠️ ELA É A TRAVA DA IDEMPOTÊNCIA, e é por isso que a lista não precisa
        // ser congelada (decisão 9): o que prende é o par `(ciclo, indicado)`.
        // Conta já existente ⇒ esta linha já foi paga em outro lote.
        let (marca_esperada, bump_marca) = Pubkey::find_program_address(
            &[PAGO_SEED, &ciclo.to_le_bytes(), linha.indicado.as_ref()],
            &crate::ID,
        );
        require_keys_eq!(
            marca_info.key(),
            marca_esperada,
            DomError::MarcaDePagoInvalida
        );
        require!(marca_info.is_writable, DomError::MarcaDePagoInvalida);
        require!(marca_info.data_is_empty(), DomError::LinhaJaPagaNesteCiclo);

        // ─── a reserva do afiliado, que carrega o que ficou abaixo do mínimo ─
        let (reserva_esperada, bump_reserva) =
            Pubkey::find_program_address(&[COMISSAO_SEED, afiliado.as_ref()], &crate::ID);
        require_keys_eq!(
            reserva_info.key(),
            reserva_esperada,
            DomError::ReservaDeComissaoInvalida
        );
        require!(
            reserva_info.is_writable,
            DomError::ReservaDeComissaoInvalida
        );
        let acumulado_antes = if reserva_info.data_is_empty() {
            0u64
        } else {
            let r: Account<ReservaDeComissao> = Account::try_from(reserva_info)?;
            require_keys_eq!(r.owner, afiliado, DomError::ReservaDeComissaoInvalida);
            r.acumulado_micro
        };

        // ─── o devido: o votado da linha mais o que estava acumulado ────────
        let devido = linha
            .micro
            .checked_add(acumulado_antes)
            .ok_or(DomError::MathOverflow)?;

        let cotas_antes = afiliado_dom.amount;

        if devido >= MIN_COMISSAO_CUNHAGEM_USDC {
            // ⚠️ ITEM 7.2: A POSIÇÃO DO AFILIADO NASCE AQUI SE NÃO EXISTIR, com o
            // rent pago por quem paga o lote. É a MESMA função que o fechamento
            // usa — ela carrega o cuidado do confisco documentado nela (posição
            // criada com `indice_entrada` errado apaga a janela de quem já tem cota).
            //
            // ⚠️ E ELA FICA **DENTRO** DO RAMO QUE PAGA. Na primeira versão vinha
            // antes do `if`, e uma linha votada em zero criava a posição e queimava
            // ~0,0011 SOL do pagador sem pagar nem acumular nada. É o mesmo cuidado
            // que o `else if devido > acumulado_antes` toma para a reserva — a Fase
            // A mediu 899.160 lamports queimados nesse descuido.
            let (mut posicao, _criada) =
                crate::instructions::deposit_especial::carregar_ou_criar_posicao(
                    afiliado_posicao_info,
                    &afiliado,
                    &ctx.accounts.vault,
                    afiliado_dom.amount,
                    &ctx.accounts.payer.to_account_info(),
                    ctx.accounts.system_program.key(),
                )?;
            // ─── PAGA: cunha ao `nav_fechamento` DO RETRATO ─────────────────
            //
            // ⚠️ O preço é o congelado, não o publicado agora. Se fosse o de
            // agora, a ordem de chegada dos lotes mudaria quantas cotas cada
            // afiliado recebe — e o oráculo publica a cada hora.
            let cotas = crate::math::shares_from_usdc(devido, nav_fechamento)?;
            let cunhadas = crate::instructions::deposit_especial::cunhar_com_acerto(
                &ctx.accounts.vault,
                &mut posicao,
                &afiliado,
                cotas_antes,
                afiliado_dom_info,
                cotas,
                nav_fechamento,
                &ctx.accounts.dom_mint.to_account_info(),
                &ctx.accounts.dom_token_program,
                vault_seeds,
            )?;
            posicao.exit(&crate::ID)?;
            cunhado_do_lote = cunhado_do_lote
                .checked_add(devido)
                .ok_or(DomError::MathOverflow)?;

            // a reserva vai a zero — ela foi paga inteira
            if acumulado_antes > 0 {
                crate::instructions::deposit_especial::gravar_reserva(
                    reserva_info,
                    &afiliado,
                    bump_reserva,
                    0,
                    &ctx.accounts.payer.to_account_info(),
                    ctx.accounts.system_program.key(),
                )?;
                // DECISOES 21:00: a reserva paga sai da dívida.
                reservas_pagas = reservas_pagas
                    .checked_add(acumulado_antes)
                    .ok_or(DomError::MathOverflow)?;
                emit!(ComissaoDaReservaPaga {
                    afiliado,
                    do_ciclo_micro: linha.micro,
                    da_reserva_micro: acumulado_antes,
                    total_micro: devido,
                    timestamp: now,
                });
            }
            // ⚠️ `bps` e `ganho_indicado` saem ZERO, e isso é informação, não
            // preguiça: no caminho do voto o contrato **não** recalcula o ganho
            // (ver o cabeçalho), então publicar um número ali seria publicar um
            // número sem fonte. Quem reconcilia usa `comissao_usdc`, que é o
            // devido de verdade, e a lista votada que a mesa registrou.
            emit!(ComissaoDeAfiliado {
                indicado: linha.indicado,
                afiliado,
                bps: 0,
                ganho_indicado: 0,
                comissao_usdc: devido,
                cotas: cunhadas,
                timestamp: now,
                cotas_comissao: cotas,
                cotas_acerto: (cunhadas as i64) - (cotas as i64),
            });
        } else if devido > acumulado_antes {
            // ─── ACUMULA: abaixo do mínimo não cunha ────────────────────────
            //
            // ⚠️ `devido > acumulado_antes`, e não só `else`: com linha votada em
            // ZERO o `else` puro criaria a conta da reserva para guardar o mesmo
            // número que já estava lá, queimando rent do bolso de quem paga.
            crate::instructions::deposit_especial::gravar_reserva(
                reserva_info,
                &afiliado,
                bump_reserva,
                devido,
                &ctx.accounts.payer.to_account_info(),
                ctx.accounts.system_program.key(),
            )?;
            // DECISOES 21:00: o que entrou na reserva é dívida nova.
            reservas_guardadas = reservas_guardadas
                .checked_add(devido - acumulado_antes)
                .ok_or(DomError::MathOverflow)?;
            emit!(ComissaoAcumulada {
                indicado: linha.indicado,
                afiliado,
                bps: 0,
                ganho_indicado: 0,
                comissao_usdc: linha.micro,
                acumulado_micro: devido,
                minimo_micro: MIN_COMISSAO_CUNHAGEM_USDC,
                timestamp: now,
            });
        }

        // ─── a marca nasce, e o gasto anda ──────────────────────────────────
        //
        // ⚠️ A marca nasce **mesmo quando a comissão foi para a reserva**: a linha
        // DAQUELE ciclo foi processada, e repeti-la pagaria duas vezes o mesmo
        // direito. O que fica pendente é a reserva, que é do afiliado e não do
        // ciclo.
        abrir_marca_de_pago(
            marca_info,
            ciclo,
            &linha.indicado,
            bump_marca,
            &ctx.accounts.payer.to_account_info(),
            ctx.accounts.system_program.key(),
        )?;
        gasto_do_lote = gasto_do_lote
            .checked_add(linha.micro)
            .ok_or(DomError::MathOverflow)?;
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 4 · O TETO DO ORÇAMENTO
    // ═══════════════════════════════════════════════════════════════════════
    // ⚠️ É ESTA a trava que substitui o recálculo do ganho. A mesa vota o valor de
    // cada linha; o contrato garante que a soma de TODOS os lotes não passa do que
    // a Fase A separou da parte da mesa.
    let gasto_depois = gasto_antes
        .checked_add(gasto_do_lote)
        .ok_or(DomError::MathOverflow)?;
    require!(gasto_depois <= orcamento, DomError::LoteAcimaDoOrcamento);

    {
        let r = &mut ctx.accounts.retrato;
        r.gasto_micro = gasto_depois;
        r.linhas_pagas = r
            .linhas_pagas
            .checked_add(linhas.len() as u16)
            .ok_or(DomError::MathOverflow)?;
        // ⚠️ DECISOES 21:00 · o contador das reservas anda aqui, e o fechamento
        // seguinte o HERDA pelo retrato anterior. `saturating_sub` porque pagar mais
        // reserva do que o contador diz é um contador que nasceu errado, e zerar é
        // melhor que estourar — mas o teste `fase_b_o_contador_de_reservas_fecha`
        // existe para que isso não aconteça em silêncio.
        r.reservas_em_aberto_micro = r
            .reservas_em_aberto_micro
            .checked_add(reservas_guardadas)
            .ok_or(DomError::MathOverflow)?
            .saturating_sub(reservas_pagas);
    }

    // -------------------------------------------------------------------------
    // ⚠️ O TETO DA JANELA SÓ SOBE SE A JANELA ESTIVER ABERTA
    // -------------------------------------------------------------------------
    // A Fase A deixou escrito o requisito de acrescentar o cunhado ao
    // `lucro_sacavel_restante`. A primeira versão disto fazia o `+` sem olhar o
    // estado do campo — e a revisão adversária PROVOU o que isso custava:
    //
    //   `lucro_sacavel_restante > 0` **é** a janela, não um teto. Um campo, três
    //   serviços (`deposit.rs:150-156` diz isso com estas palavras):
    //     · `deposit`/`deposit_para` RECUSAM aporte com a janela aberta;
    //     · `execute` recusa transferência de cota para destino não isento;
    //     · `sacar_lucro` passa a deixar sacar.
    //
    //   O `deploy_capital` ZERA o campo e publica `JanelaDeLucroEncerrada`. Um lote
    //   depois dele REABRIA a janela: medido pela sonda da revisão, o aporte passava
    //   antes do lote e recusava com `JanelaDeLucroAberta` depois. Com 42 linhas em
    //   lotes de 4 a 6, cada um um voto 2-de-3, lote depois do `deploy_capital` é o
    //   caso NORMAL — o fundo fechava para dinheiro novo por causa de uma comissão.
    //
    // ⓘ E por que o `+` não é necessário para o afiliado: a cunhagem registra a
    // entrada dele no índice DE AGORA (`cunhar_com_acerto`), então o ganho dele pelo
    // ciclo que fechou é ZERO — ele não alcança a janela. O caso em que o teto
    // importa é o afiliado que JÁ tinha cota e posição antiga, e aí a janela está
    // aberta de qualquer forma.
    //
    // ⚠️ E o `.min(p)` que o requisito da Fase A pede NÃO É EXEQUÍVEL AQUI: o
    // `p_ciclo` vai a zero no fechamento e o retrato **não guarda `p`** (dez campos,
    // conferidos). Então o teto pode passar de `p` pelo que vier de reserva de ciclo
    // ANTERIOR — até ~1 USDC por afiliado. Está declarado no `ATUAL.md` como
    // pergunta ao dono, e não tapado aqui.
    if cunhado_do_lote > 0 {
        let v = &mut ctx.accounts.vault;
        if v.lucro_sacavel_restante > 0 {
            v.lucro_sacavel_restante = v
                .lucro_sacavel_restante
                .checked_add(cunhado_do_lote)
                .ok_or(DomError::MathOverflow)?;
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 5 · O LOTE QUE FECHA: a sobra volta à mesa em COTAS (decisão 6)
    // ═══════════════════════════════════════════════════════════════════════
    let mut sobra_em_cotas = 0u64;
    let mut sobra_micro = 0u64;
    if fecha_o_ciclo {
        let sobra = orcamento.saturating_sub(gasto_depois);
        let base = linhas.len() * CONTAS_POR_LINHA_DO_LOTE;
        let socios = ctx.accounts.vault.socios;
        let (c, m) = devolver_sobra_a_mesa(
            &ctx.accounts.vault,
            &socios,
            sobra,
            nav_fechamento,
            &ctx.remaining_accounts[base..],
            &ctx.accounts.dom_mint,
            &ctx.accounts.dom_token_program,
            vault_seeds,
        )?;
        sobra_em_cotas = c;
        sobra_micro = m;
        let r = &mut ctx.accounts.retrato;
        // ⚠️ `gasto_micro` GUARDA O QUE FOI PAGO EM LINHAS, e não o orçamento.
        //
        // A primeira versão gravava `gasto = orcamento` para dizer "a dívida
        // fechou" — e apagava do ESTADO a única prova durável do rateio (quanto
        // foi para as linhas contra quanto voltou como sobra), deixando só o
        // evento. A casa escolheu o contrário no DV12: *"a atestação fica no
        // estado, não só no evento — evento expira da janela de retenção do RPC"*.
        //
        // Quem diz que não há mais dívida é o `aberto`, e o doc dele no `state.rs`
        // já descreve os dois casos legítimos de `false`.
        r.aberto = false;
    }

    emit!(LoteDeComissaoPago {
        ciclo,
        linhas_do_lote: linhas.len() as u16,
        linhas_pagas: ctx.accounts.retrato.linhas_pagas,
        linhas_votadas: ctx.accounts.retrato.linhas_votadas,
        gasto_do_lote,
        gasto_total: ctx.accounts.retrato.gasto_micro,
        orcamento_micro: orcamento,
        cunhado_micro: cunhado_do_lote,
        sobra_micro,
        sobra_em_cotas,
        fechou: fecha_o_ciclo,
        timestamp: now,
    });
    Ok(())
}

/// Abre a marca `["pago", ciclo, indicado]`.
///
/// ⚠️ Ela guarda o ciclo e o indicado **de propósito**, mesmo sendo derivável da
/// semente: quem lê a conta pela cadeia (a vista, o painel, uma auditoria) não
/// tem como inverter a semente, e uma marca que não diz o que marca é um byte
/// opaco. O custo é 48 bytes de rent.
fn abrir_marca_de_pago<'info>(
    info: &'info AccountInfo<'info>,
    ciclo: u64,
    indicado: &Pubkey,
    bump: u8,
    payer: &AccountInfo<'info>,
    system_program_id: Pubkey,
) -> Result<()> {
    let espaco = 8 + crate::state::MarcaDePago::INIT_SPACE;
    let seeds: &[&[u8]] = &[PAGO_SEED, &ciclo.to_le_bytes(), indicado.as_ref(), &[bump]];
    crate::instructions::deposit_especial::abrir_conta_do_programa(
        info,
        seeds,
        espaco,
        payer,
        system_program_id,
    )?;
    let marca = crate::state::MarcaDePago {
        ciclo,
        indicado: *indicado,
        bump,
    };
    let mut dados = info.try_borrow_mut_data()?;
    let mut cursor = std::io::Cursor::new(&mut dados[..]);
    marca.try_serialize(&mut cursor)?;
    Ok(())
}

/// ─── A SOBRA DA MESA VOLTA EM COTAS · UM LUGAR SÓ ───────────────────────────
///
/// ⚠️ **Por que isto é função e não código repetido:** a regra do dono (DECISOES
/// 13/10 16:55, item 7) diz que a sobra volta à mesa *"automaticamente no lote que
/// fizer `linhas_pagas = linhas_votadas`, **ou no `encerrar_ciclo`**"*. São DUAS
/// portas para o MESMO dinheiro, e duas cópias da mesma aritmética divergiriam —
/// a casa já pagou por isso (o `.max(nav_fechamento)` que vivia em um lugar e não
/// no outro). As duas portas chamam esta função.
///
/// `contas` são as SEIS contas da mesa, nesta ordem: as 3 ATAs de cota dos sócios
/// e as 3 posições, na ordem de `vault.socios`. Devolve `(cotas, micro)` — o que
/// foi CUNHADO e o que foi RATEADO, que não são o mesmo número.
#[allow(clippy::too_many_arguments)]
pub(crate) fn devolver_sobra_a_mesa<'info>(
    vault: &Account<'info, Vault>,
    socios: &[Pubkey; NUM_SOCIOS],
    sobra: u64,
    nav_fechamento: u64,
    contas: &'info [AccountInfo<'info>],
    dom_mint: &InterfaceAccount<'info, Mint>,
    dom_token_program: &Interface<'info, TokenInterface>,
    vault_seeds: &[&[&[u8]]],
) -> Result<(u64, u64)> {
    require!(
        contas.len() >= NUM_SOCIOS * 2,
        DomError::ContasDaMesaFaltando
    );
    let mut sobra_em_cotas = 0u64;
    let mut sobra_micro = 0u64;
    if sobra > 0 {
        // ⚠️ Ao `nav_fechamento` DO RETRATO, e não ao NAV de agora: é o mesmo
        // preço com que a mesa teria cunhado no fechamento se não houvesse
        // custódia. Usar o de agora faria a mesa ganhar ou perder pela demora
        // dos lotes.
        let por_socio = sobra / NUM_SOCIOS as u64;
        for (i, socio) in socios.iter().enumerate().take(NUM_SOCIOS) {
            let dom_info = &contas[i];
            let pos_info = &contas[NUM_SOCIOS + i];
            let conta: InterfaceAccount<TokenAccount> = InterfaceAccount::try_from(dom_info)?;
            require_keys_eq!(conta.owner, *socio, DomError::ContaDeCotaNaoEAta);
            require_keys_eq!(conta.mint, dom_mint.key(), DomError::UnknownMint);
            crate::indice::exigir_ata(
                &dom_info.key(),
                socio,
                &dom_mint.key(),
                &dom_token_program.key(),
            )?;
            let mut pos: Account<PosicaoDoCotista> =
                crate::instructions::deposit_especial::carregar_posicao(pos_info, socio, true)?;
            let cotas = crate::math::shares_from_usdc(por_socio, nav_fechamento)?;
            let cunhadas = crate::instructions::deposit_especial::cunhar_com_acerto(
                vault,
                &mut pos,
                socio,
                conta.amount,
                dom_info,
                cotas,
                nav_fechamento,
                &dom_mint.to_account_info(),
                dom_token_program,
                vault_seeds,
            )?;
            pos.exit(&crate::ID)?;
            // ⚠️ O EVENTO DIZ O QUE FOI CUNHADO, NÃO O QUE SE PRETENDIA.
            //
            // A primeira versão somava `por_socio` (micro-USDC) e chamava isso
            // de "sobra devolvida". Duas mentiras, as duas medidas pela revisão
            // adversária:
            //   · `shares_from_usdc` TRUNCA (a favor do cofre, como manda a
            //     casa): com sobra de 3 micro, `por_socio = 1` e as cotas são
            //     **zero** — o evento declarava 3 e nada foi para a mesa;
            //   · `cunhar_com_acerto` devolve o que de FATO cunhou (o acerto do
            //     sócio entra netado), e o retorno era ignorado. O laço da
            //     própria Fase A soma o retorno, não o pretendido.
            sobra_em_cotas = sobra_em_cotas
                .checked_add(cunhadas)
                .ok_or(DomError::MathOverflow)?;
            sobra_micro = sobra_micro
                .checked_add(por_socio)
                .ok_or(DomError::MathOverflow)?;
        }
        // ⓘ O QUE FICA COM OS COTISTAS, e são DUAS coisas, não uma:
        //   · o resto da divisão por 3 (até 2 micro);
        //   · e o truncamento de `shares_from_usdc` em cada sócio — que é
        //     maior que o resto e tem outra causa. Com `nav > 1` ele é
        //     inevitável, e é o arredondamento a favor do cofre que a casa
        //     manda. A primeira versão deste comentário dizia só o resto.
    } else {
        // Sem sobra, as 6 contas ainda vêm (a contagem é a mesma) e são só
        // conferidas: isso mantém o cliente que monta a proposta com uma regra
        // só — "o lote que fecha leva as contas da mesa".
        for (i, socio) in socios.iter().enumerate().take(NUM_SOCIOS) {
            let dom_info = &contas[i];
            crate::indice::exigir_ata(
                &dom_info.key(),
                socio,
                &dom_mint.key(),
                &dom_token_program.key(),
            )?;
        }
    }
    Ok((sobra_em_cotas, sobra_micro))
}
