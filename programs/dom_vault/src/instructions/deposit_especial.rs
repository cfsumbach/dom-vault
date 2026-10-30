use {
    crate::{
        constants::*,
        error::DomError,
        events::{
            ComissaoAcumulada, ComissaoDaReservaPaga, ComissaoDeAfiliado, JanelaDeLucroEncerrada,
            LucroDistribuido,
        },
        math::{require_nav_fresco, shares_from_usdc},
        state::{FeeShareLedger, PosicaoDoCotista, ReservaDeComissao, RetratoDoCiclo, Vault},
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
/// Esta instrução **abre** a janela: grava `lucro_sacavel_restante = P − mesa`
/// (⚠️ dizia "40% de P", o que descreveria `perf_fee_bps_total = 6000`. Em
/// produção a taxa é **5000**, e o mínimo do contrato é 5000
/// (`MAX_EXCEDENTE_POEIRA, MIN_PERF_FEE_BPS_TOTAL`), então a parcela dos cotistas é **50% de P** mais a
/// sobra da divisão por três. Comentário com número fixo envelhece quando o
/// número mora no estado — corrigido em 06/10, na conferência do ciclo 3.)
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
/// de existir (`abrir_posicao`).
///
/// ⚠️ UPGRADE N · ITEM 6 — ESTE COMENTARIO DIZIA "sem whitelist para sócio", e a
/// frase sugeria uma ISENCAO que o socio nao tem. Nao ha isencao nenhuma:
/// **a whitelist nao e consultada em cunhagem nenhuma**, para ninguem.
///
/// Medido: `execute.rs:88-95` exige `is_transferring` na ORIGEM **e** no
/// DESTINO antes de chegar ao `require_whitelisted` de `:112`/`:115`. Um
/// `MintTo` nao tem conta de origem no sentido do hook — **entao o hook nao e
/// chamado** e a whitelist nao entra na conta. O mesmo vale para o
/// `sacar_lucro`, que paga USDC: nao e transferencia de cota, nao ha hook.
///
/// Decisao do dono (08/10): *"todo destino passa — afiliado, cotista, mesa,
/// transferencia, sacar_lucro. Nao perguntar 'e se estiver fora'."*
///
/// ⚠️ E "todo destino passa" NAO quer dizer "a whitelist esta desligada": ela
/// continua barrando TRANSFERENCIA de cota entre carteiras, que e' o caminho em
/// que o hook roda. A diferenca e' o caminho, nao o privilegio de quem recebe.
/// O `tests/whitelist_nao_barra_cunhagem.rs` trava os dois lados.
///
/// `[3..]` (J1) por linha da lista `afiliados`, na ordem dela, quatro contas:
/// `indicado_dom` (conta de cota do indicado — só leitura, dá as cotas),
/// `indicado_posicao` (a entrada dele), `afiliado_dom` (recebe a comissão,
/// mutável), `afiliado_posicao` (mutável: a comissão é uma entrada e o acerto
/// dele é compensado na cunhagem, como o dos sócios).
#[derive(Accounts)]
#[instruction(lucro_realizado: u64, ciclo: u64)]
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

    /// CHECK: aberto ou carregado no corpo (`somar_no_livro`): PDA, dono e
    /// discriminador.
    #[account(mut)]
    pub socio0_ledger: UncheckedAccount<'info>,
    /// CHECK: aberto ou carregado no corpo (`somar_no_livro`): PDA, dono e
    /// discriminador.
    #[account(mut)]
    pub socio1_ledger: UncheckedAccount<'info>,
    /// CHECK: aberto ou carregado no corpo (`somar_no_livro`): PDA, dono e
    /// discriminador.
    #[account(mut)]
    pub socio2_ledger: UncheckedAccount<'info>,

    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
    pub usdc_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    // -----------------------------------------------------------------------
    // FASE A DO FECHAMENTO EM LOTES (aprovado pelo dono em 13/10 16:55)
    // -----------------------------------------------------------------------
    /// O retrato deste ciclo — nasce aqui, e é o que a Fase B vai ler.
    ///
    /// ⚠️ `init` e não `init_if_needed`: fechar o mesmo ciclo duas vezes é a
    /// anomalia que esta conta existe para impedir. A segunda tentativa falha na
    /// criação, alto, antes de mover um micro.
    #[account(
        init,
        payer = payer,
        space = 8 + RetratoDoCiclo::INIT_SPACE,
        seeds = [RETRATO_SEED, &ciclo.to_le_bytes()],
        bump,
    )]
    pub retrato: Box<Account<'info, RetratoDoCiclo>>,
    /// O retrato do ciclo ANTERIOR — conferido, nunca escrito.
    ///
    /// ⚠️ **Ele é a corrente que impede ciclo votado errado.** O cofre não tem
    /// contador de ciclo (`state.rs:80`) e o número vem votado; o que o prende é
    /// exigir que o anterior **exista, seja a PDA de `ciclo − 1` e esteja
    /// fechado**. O primeiro ciclo com retrato (`PRIMEIRO_CICLO_DO_RETRATO`) não
    /// tem antecessor, e aí esta conta vem **vazia** — e só nesse caso.
    ///
    /// `UncheckedAccount` porque ela pode legitimamente não existir; a conferência
    /// é no corpo, e confere **os três**: a derivação, o dono e o discriminador.
    /// CHECK: conferido no corpo (`conferir_retrato_anterior`).
    pub retrato_anterior: UncheckedAccount<'info>,
}

pub fn handle_deposit_especial<'info>(
    ctx: Context<'info, DepositEspecial<'info>>,
    lucro_realizado: u64,
    // -----------------------------------------------------------------------
    // FASE A: os tres numeros que a mesa VOTA
    // -----------------------------------------------------------------------
    // ⚠️ `ciclo` vem ANTES do `Vec` por uma razao de formato, nao de estilo: a
    // semente da PDA do retrato le' `ciclo` pelo `#[instruction(...)]`, e o Anchor
    // desserializa um PREFIXO dos argumentos. Com o `Vec` na frente, o prefixo
    // teria de atravessar um vetor de tamanho variavel.
    //
    // `ciclo`: o cofre nao tem contador (`state.rs:80`) e nao ha byte livre para
    // criar um. A corrente dos retratos e' o que impede numero errado de passar.
    //
    // `linhas_votadas`: so' a CONTAGEM (decisao 9 do dono) — congelar 20 pares de
    // pubkeys nao cabe em transacao, e provar pertinencia depois pediria Merkle.
    //
    // `orcamento_comissoes_micro`: a custodia. Votar a MAIS e' seguro (a sobra
    // volta a' mesa em cotas); votar a MENOS deixa linha sem pagar, e isso APARECE
    // em `linhas_pagas < linhas_votadas`.
    ciclo: u64,
    linhas_votadas: u16,
    orcamento_comissoes_micro: u64,
    // ⚠️ Upgrade M: so o `bps` por linha. As pubkeys saem das CONTAS.
    afiliados: Vec<u16>,
) -> Result<()> {
    // -----------------------------------------------------------------------
    // ⚠️ OS DOIS CAMINHOS COEXISTEM, E SAO MUTUAMENTE EXCLUSIVOS
    // -----------------------------------------------------------------------
    // Este commit e' a FASE A. A Fase B (`pagar_comissoes`) ainda nao existe, e
    // derrubar o laco dos afiliados agora deixaria o programa sem pagar comissao
    // nenhuma entre dois commits.
    //
    // Entao vale o padrao da casa (`dom-migracao`): **antigo e novo em paralelo, e
    // o antigo morre quando o novo existe**. Quem escolhe e' a mesa, pelo voto:
    //
    //   orcamento == 0  ->  o laco paga as comissoes aqui (o caminho de hoje), e o
    //                       retrato nasce FECHADO: nao ha nada para a Fase B pagar
    //   orcamento  > 0  ->  a lista TEM de vir vazia; o valor fica em custodia, o
    //                       retrato nasce ABERTO e a Fase B paga em lotes
    //
    // ⚠️ Os dois juntos debitariam a mesa duas vezes — por isso recusa.
    // ⚠️ NAO PEDIDO: a coexistencia e' minha. A morte do caminho antigo entra na
    //    definicao de pronto da Fase B.
    require!(
        orcamento_comissoes_micro == 0 || afiliados.is_empty(),
        DomError::OrcamentoComListaDeAfiliados
    );
    // ⚠️ E as DUAS pontas do mesmo defeito, cada uma com o seu erro:
    //
    //   linha sem dinheiro  -> um retrato dizendo "5 linhas a pagar, 0 para pagar
    //                          com": a Fase B nunca fecharia esse ciclo;
    //   dinheiro sem linha  -> custodia com ninguem a receber. O retrato nasceria
    //                          ABERTO e, pela decisao 7, a sobra so volta no lote
    //                          que faz `linhas_pagas == linhas_votadas` — com zero
    //                          linha nunca ha lote, e o ciclo seguinte TRAVA.
    //
    // A segunda passava. Achado da revisao adversaria de 13/10.
    require!(
        orcamento_comissoes_micro > 0 || linhas_votadas == 0,
        DomError::LinhaVotadaSemOrcamento
    );
    require!(
        orcamento_comissoes_micro == 0 || linhas_votadas > 0,
        DomError::OrcamentoSemLinhaVotada
    );
    // ⚠️ E a corrente dos ciclos: nao existe retrato antes do primeiro.
    require!(
        ciclo >= PRIMEIRO_CICLO_DO_RETRATO,
        DomError::CicloAntesDoPrimeiroRetrato
    );
    conferir_retrato_anterior(&ctx.accounts.retrato_anterior, ciclo, ctx.program_id)?;
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
    //
    // ⚠️ O PONTO ZERO, CAPTURADO **ANTES** DE ABSORVER. Esta e' a correcao do
    // Upgrade L, e ela destrava o primeiro fechamento do J.
    //
    // `set_gaveta_usdc` grava `gaveta_saldo_visto = gaveta.amount` e NAO soma esse
    // saldo a `p_ciclo`, de proposito: "o saldo que a conta ja tem ao ser apontada
    // nao e P: e o ponto zero" (gaveta.rs). Mas a conservacao comparava
    // `p_ciclo == p`, e `p` e o saldo INTEIRO da conta — que inclui o ponto zero
    // que `p_ciclo` exclui. Com ponto zero diferente de zero, a trava era
    // impossivel de satisfazer e o ciclo nao fechava NUNCA.
    //
    // Em producao o ponto zero e 1000 micro (0,001 USDC): o
    // `PROCEDIMENTO-UPGRADE`, anexo J, registra "9ecvK773 e a ATA canonica do
    // vault 1, unica conta dele, 0,001 USDC" — era o saldo da gaveta quando a
    // migracao do J a apontou.
    //
    // ⚠️ NAO PRECISA DE CAMPO NOVO, e isso importa: a conta tem 731 bytes e os
    // 723 uteis estao TODOS ocupados — campo novo exigiria realloc e migracao.
    // O ponto zero e DERIVAVEL: `z = gaveta_saldo_visto - p_ciclo`. A varredura
    // de todo escritor dos dois campos prova a invariante:
    //
    //   initialize / migracao_j  ... visto=0, p_ciclo=0        -> z = 0
    //   set_gaveta_usdc .......... visto=amount, p_ciclo==0    -> z = amount
    //   absorver_no_cofre ........ os dois +delta              -> z inalterado
    //   este fechamento (abaixo) . visto=0, p_ciclo=0          -> z = 0
    //
    // ⚠️ E TEM DE SER ANTES DO `absorver_no_cofre`: ele grava
    // `gaveta_saldo_visto = p`, e dai `z` viraria `p - p_ciclo`, a trava viraria
    // `p_ciclo == p_ciclo` — tautologia, e a conservacao perderia os dentes.
    let gaveta_key = ctx.accounts.gaveta.key();
    let ponto_zero = ctx
        .accounts
        .vault
        .gaveta_saldo_visto
        .saturating_sub(ctx.accounts.vault.p_ciclo);
    let saldo = ctx.accounts.gaveta.amount;
    crate::indice::absorver_no_cofre(&mut ctx.accounts.vault, &gaveta_key, saldo, supply)?;
    // ⚠️ UPGRADE L · O VOTO E UM PISO, NAO UMA IGUALDADE.
    //
    // Era `require!(p == lucro_realizado)`, com `p` lido da conta na execucao. Isso
    // dava a QUALQUER UM na rede o poder de derrubar a proposta: mandar 0,0001
    // USDC entre o voto e a execucao, e o saldo deixa de casar com o numero
    // votado. Revotar nao resolve — a proxima poeira derruba a proxima proposta.
    // Negacao de servico permanente no fechamento do ciclo, por fracoes de
    // centavo por tentativa.
    //
    // Nao e hipotese: em 05/10 chegaram QUATRO poeiras na gaveta, de dois
    // enderecos vanity construidos sobre a Operacional do fundo — um com o sufixo
    // IDENTICO —, 23 s e 5 s depois de transferencias nossas. O mesmo atacante
    // poeirou o colchao do operador. O alvo dele era o humano que copia endereco
    // do historico; a trava do contrato foi dano colateral.
    //
    // Agora: o saldo tem de COBRIR o voto, e o que se distribui e' o VOTO.
    require!(saldo >= lucro_realizado, DomError::GavetaAbaixoDoVotado);
    // ⚠️ E O EXCEDENTE TEM TETO. `>=` puro deixaria quem propoe votar 1 micro: o
    // ciclo fecharia, o relogio de 15 dias reiniciaria, e a janela anterior nao
    // sacada seria ZERADA pelo `JanelaDeLucroEncerrada` — distribuindo quase nada.
    // Com o teto, poeira passa e sub-votar nao.
    require!(
        saldo - lucro_realizado <= MAX_EXCEDENTE_POEIRA,
        DomError::PDiferenteDoEsperado
    );
    // ⚠️ `p` passa a ser O QUE A MESA VOTOU, e e' deliberado que o nome continue
    // `p`: tudo abaixo — a parcela da mesa, a dos cotistas, a transferencia para o
    // treasury, o piso e o evento — passa a operar sobre o votado sem que eu tenha
    // de acertar cinco pontos a mao e esquecer o sexto.
    let p = lucro_realizado;
    require!(p > 0, DomError::ZeroLucro);
    // ⚠️ O QUE ESTA TRAVA E, EXATAMENTE — e eu quase commitei a justificativa
    // errada, dizendo que ela "continua com dentes contra ataque". Nao e isso, e
    // nunca foi. A algebra:
    //
    //   z              = visto_antes - p_ciclo_antes
    //   delta          = p - visto_antes            (dentro do absorver)
    //   p_ciclo_depois = p_ciclo_antes + delta
    //                  = p_ciclo_antes + p - visto_antes
    //                  = p - z
    //
    // Entao `p_ciclo == p - z` e SEMPRE verdade quando a invariante vale. E a
    // forma antiga, `p_ciclo == p`, era igualmente tautologica quando z era 0 —
    // e IMPOSSIVEL quando z nao era. Ela nunca guardou o dinheiro contra ninguem.
    //
    // O que ela e': uma ASSERCAO DE INVARIANTE. Ela dispara se um escritor futuro
    // mexer em `gaveta_saldo_visto` sem mexer em `p_ciclo` (ou o contrario) — o
    // unico jeito de os dois descolarem. E esse e' um acidente plausivel: foi
    // exatamente o que o `set_gaveta_usdc` fez de proposito, e a trava o leu como
    // corrupcao. Vale manter, com a conta certa e dizendo o que ela mede.
    //
    // ⚠️ O `p` que vai para o treasury continua sendo o saldo INTEIRO, ponto zero
    // incluido: o dinheiro e do fundo e nao pode ficar encalhado na gaveta. Entao
    // o ponto zero e distribuido como se fosse lucro — 0,001 USDC em producao,
    // rateado entre mesa e cotistas na mesma proporcao de todo o resto. Deixa-lo
    // na gaveta seria pior: `gaveta_saldo_visto` zera aqui, e o resto viraria P do
    // ciclo seguinte sem nunca ter sido ganho de ninguem.
    require!(
        ctx.accounts.vault.p_ciclo == saldo.saturating_sub(ponto_zero),
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
    // ⚠️ DECISOES 13/10 21:00: a RESERVA do afiliado tambem e DIVIDA e sai do piso.
    // Este e o delta do ciclo: o que foi guardado menos o que foi pago. O total em
    // aberto e o do retrato ANTERIOR mais este delta.
    let mut reservas_guardadas: u64 = 0;
    let mut reservas_pagas: u64 = 0;

    // 3 · (N7) os afiliados: comissao_i = ganho_indicado × bps.
    //
    // ⚠️ O `taxa_mesa` SAIU DA CONTA em 12/10/2026, por decisao do dono: os bps
    // do afiliado incidem sobre o LUCRO BRUTO do indicado, nao sobre a parte da
    // mesa. Com 500 bps e perf_fee de 5.000, a conta antiga pagava 2,5% do bruto
    // — metade do prometido. De 100 de lucro: cotista 50 · mesa 45 · afiliado 5.
    //
    // O ganho do indicado e' o do ciclo que acabou de fechar — pelo indice,
    // com as cotas que ele tem agora (a mesma base do acerto dele). A comissao
    // continua saindo da mesa: o cotista nao paga um micro a mais, e a trava
    // `comissoes_total <= parcela_socios` (abaixo) e' o que garante isso.
    require!(
        ctx.remaining_accounts.len() >= NUM_SOCIOS + afiliados.len() * CONTAS_POR_AFILIADO,
        DomError::AfiliadoInvalido
    );
    let mut comissoes_total: u64 = 0;
    // ITEM 7: o que de fato VIROU COTA nesta instrucao (devido do ciclo +
    // reserva paga). Diferente de `comissoes_total`, que e' o DEVIDO do ciclo e
    // e' o que a mesa paga. Os dois sao necessarios: um fecha o debito da mesa,
    // o outro o teto da janela.
    let mut comissoes_cunhadas: u64 = 0;
    // ⚠️ NAO HA CONTADOR DE POSICOES CRIADAS. A primeira versao tinha um
    // `posicoes_criadas` incrementado e NUNCA LIDO — e este arquivo diz, 150
    // linhas abaixo, que "variavel viva que ninguem le e' a proxima a ser lida por
    // engano". Publicar o numero exigiria campo novo em evento, e evento desta
    // casa se decodifica POR ORDEM: o backend quebraria. Fica declarado no
    // relatorio do item, nao numa variavel morta.
    //
    // ─── ⚠️ UPGRADE M · AS PUBKEYS SAEM DAS CONTAS, NAO DO ARGUMENTO ──────────
    //
    // O argumento era `Vec<{ indicado: Pubkey, afiliado: Pubkey, bps: u16 }>` —
    // 66 bytes por linha, dos quais 64 eram as duas pubkeys. E elas JA VINHAM NAS
    // CONTAS: este laco sempre conferiu `indicado_dom.owner == indicado`.
    // O argumento repetia o que as contas carregavam.
    //
    // ⚠️ E ISSO TRAVOU O FECHAMENTO DO CICLO 3 EM MAINNET, em 07/10. Medido:
    //
    // data da instrucao com 20 afiliados ... 1340 bytes
    // mensagem interna do Squads ........... 1583 bytes
    // uma transacao Solana ................. 1232 bytes
    //
    // Sem saida por ALT: ALT comprime CONTA, nunca ARGUMENTO. E sem saida pelo
    // `transactionBuffer` do Squads: as tres escritas passaram, o buffer ficou
    // perfeito (hash conferido, desserializavel), e o
    // `vaultTransactionCreateFromBuffer` do programa no ar recusou com
    // `BorshIoError("Unexpected length of input")` — ele nao substitui o
    // argumento pelo conteudo do buffer.
    //
    // Com `Vec<u16>`: 2 bytes por linha, 60 de data, mensagem ~300. Cabe numa
    // transacao so, sem buffer e sem ALT extra.
    //
    // ⚠️ E A VALIDACAO FICA MAIS FORTE, NAO MAIS FRACA. Antes o contrato RECEBIA
    // as pubkeys e as comparava com as contas; agora ele as DERIVA das contas.
    // Argumento que nao existe nao pode mentir, e uma comparacao a menos e uma
    // forma a menos de errar.
    //
    // ═══════════════════════════════════════════════════════════════════════
    // PASSO 1 · LER E CONFERIR TUDO, **ANTES** DE CUNHAR UMA COTA
    // ═══════════════════════════════════════════════════════════════════════
    //
    // ⚠️ **A ORDEM DA LISTA MUDAVA O DINHEIRO, e isso foi MEDIDO duas vezes em
    // 13/10** — pelo devnet em LiteSVM (`dom-arquitetura · dev · 8521293`:
    // +24.418 micro de ganho e +1.220 micro de comissão paga pela mesa, com o
    // saque da carteira saindo ~12.000 micro inflado, pago pelos outros
    // cotistas) e, em paralelo, por uma revisão adversária com outra permutação
    // (+99.646 micro).
    //
    // A causa: uma carteira que é **afiliado numa linha e indicado em outra**.
    // O laço lia `indicado_dom.amount` e `indicado_posicao.indice_entrada`
    // DENTRO dele, depois de as linhas anteriores já terem cunhado — então as
    // cotas de comissão que ela acabou de receber contavam como ganho do ciclo
    // que já havia fechado, e o afiliado seguinte cobrava bps sobre elas.
    //
    // Ordem do dono (13/10 14:00): *"ler o ganho de cada indicado e a entrada de
    // cada carteira do estado ANTES de qualquer cunhagem do fechamento, e só
    // depois cunhar"*. É o que este passo faz: nada abaixo escreve.
    //
    // ⚠️ **E uma recusa NOVA aparece de graça, de propósito:** se a carteira X é
    // afiliado numa linha e indicado em outra e **não tem posição**, a posição
    // dela deixa de ser criada antes da leitura da outra linha — e o
    // `carregar_posicao` do indicado recusa (`PosicaoDoCotistaAusente`). Antes,
    // aquela leitura devolvia ganho ZERO em silêncio. Recusar alto é melhor que
    // calcular baixo.
    struct LinhaLida {
        bps: u16,
        indicado: Pubkey,
        afiliado: Pubkey,
        /// O ganho do indicado no ciclo fechado, fotografado ANTES de qualquer
        /// cunhagem desta instrução.
        ganho_indicado: u64,
    }
    let mut lidas: Vec<LinhaLida> = Vec::with_capacity(afiliados.len());
    for (n, bps) in afiliados.iter().copied().enumerate() {
        require!(
            (1..=MAX_AFILIADO_BPS).contains(&bps),
            DomError::AfiliadoInvalido
        );
        let base = NUM_SOCIOS + n * CONTAS_POR_AFILIADO;
        let indicado_dom =
            InterfaceAccount::<TokenAccount>::try_from(&ctx.remaining_accounts[base])?;
        let afiliado_dom =
            InterfaceAccount::<TokenAccount>::try_from(&ctx.remaining_accounts[base + 2])?;
        //
        // As duas contas TEM de ser do mint do cofre: sem isto, `owner` de uma
        // conta de outro mint entraria como se fosse carteira de cotista.
        //
        require!(
            indicado_dom.mint == ctx.accounts.dom_mint.key()
                && afiliado_dom.mint == ctx.accounts.dom_mint.key()
                && ctx.remaining_accounts[base + 2].is_writable,
            DomError::AfiliadoInvalido
        );
        // ⚠️ DERIVADAS, nao recebidas.
        let indicado = indicado_dom.owner;
        let afiliado = afiliado_dom.owner;
        require!(indicado != afiliado, DomError::AfiliadoInvalido);
        // -------------------------------------------------------------------
        // ⚠️ A ATA CANÔNICA, E ISTO ERA UM CONFISCO
        // -------------------------------------------------------------------
        // `mint == dom_mint` NÃO basta. Uma segunda conta de cota do MESMO dono,
        // no MESMO mint, passava por aqui — e `indice::exigir_ata` (`indice.rs:438`)
        // diz, por escrito, que **toda porta que lê cota para o índice exige a
        // ATA canônica**: *"se o dono pudesse guardar cota numa segunda conta,
        // acertaria pela menor, marcaria a posição como em dia e a maior
        // escaparia da taxa"*. Esta instrução era a porta que não exigia.
        //
        // ⚠️ **Medido pela revisão adversária de 13/10**, com uma conta auxiliar
        // vazia do mesmo dono como `afiliado_dom`: `cotas_antes` foi lido como
        // **0**, a entrada ponderada jogou `indice_entrada` para o índice de
        // agora, e o direito do afiliado — que também era cotista — no ciclo que
        // acabou de fechar virou **ZERO** (≈ 240 USDC de saque no cenário dela).
        // Pela mesma porta, a guarda `cotas_do_afiliado == 0` do item 7.2 — a que
        // o comentário de `carregar_ou_criar_posicao` chama de "O CONFISCO QUE
        // ESTA FUNÇÃO NÃO COMETE" — era furada: basta passar a conta vazia.
        //
        // O erro já existia (`ContaDeCotaNaoEAta`), e as outras três portas já o
        // usavam. Só esta estava aberta.
        crate::indice::exigir_ata(
            &ctx.remaining_accounts[base].key(),
            &indicado,
            &ctx.accounts.dom_mint.key(),
            &ctx.accounts.dom_token_program.key(),
        )?;
        crate::indice::exigir_ata(
            &ctx.remaining_accounts[base + 2].key(),
            &afiliado,
            &ctx.accounts.dom_mint.key(),
            &ctx.accounts.dom_token_program.key(),
        )?;
        //
        // ⚠️ O REPETIDO É CONFERIDO PELAS CONTAS JÁ VISTAS, e não pelo argumento —
        // que não tem mais a pubkey.
        //
        // ⓘ **E agora pela fotografia, não re-desserializando as contas.** A
        // versão anterior relia cada `indicado_dom` anterior a cada linha: 55
        // desserializações para 11 linhas, O(n²) de CU, para chegar ao mesmo
        // `owner` que o passo 1 já derivou da MESMA conta nesta mesma corrida. A
        // garantia é idêntica (a fonte é a conta, não o argumento) e o custo cai
        // para comparação de pubkey.
        //
        // Medido: o fechamento com 3 sócios + 3 afiliados estava a 174.671 CU, a
        // 13% do teto padrão de 200.000 — e o portão da casa exige 15% de margem.
        for lida in lidas.iter() {
            require!(lida.indicado != indicado, DomError::AfiliadoRepetido);
        }
        let indicado_posicao =
            carregar_posicao(&ctx.remaining_accounts[base + 1], &indicado, false)?;
        let v = &ctx.accounts.vault;
        let ganho_indicado = crate::indice::ganho(
            indicado_dom.amount,
            indicado_posicao.indice_entrada,
            v.indice_ciclo_anterior,
            v.indice_ciclo,
        )?;
        lidas.push(LinhaLida {
            bps,
            indicado,
            afiliado,
            ganho_indicado,
        });
    }

    // ═══════════════════════════════════════════════════════════════════════
    // PASSO 2 · CUNHAR, com os números já fotografados
    // ═══════════════════════════════════════════════════════════════════════
    for (n, linha) in lidas.iter().enumerate() {
        let (bps, indicado, afiliado, ganho_indicado) = (
            linha.bps,
            linha.indicado,
            linha.afiliado,
            linha.ganho_indicado,
        );
        let base = NUM_SOCIOS + n * CONTAS_POR_AFILIADO;
        // ⚠️ O saldo de cota do afiliado é relido **FRESCO** aqui, e isso é
        // deliberado: o DINHEIRO (o ganho, logo a comissão) sai da fotografia do
        // passo 1 e por isso não depende da ordem; a conta de `indice_entrada`,
        // porém, é uma média ponderada CUMULATIVA — se a mesma carteira recebe em
        // duas linhas, a segunda precisa ver as cotas que a primeira cunhou, ou a
        // entrada dela sairia errada. Uma coisa é o valor devido; outra é o preço
        // médio de entrada de quem recebeu.
        let afiliado_dom =
            InterfaceAccount::<TokenAccount>::try_from(&ctx.remaining_accounts[base + 2])?;
        // ITEM 7.2: a posicao do afiliado e criada AQUI se nao existir, com o
        // rent pago pela mesa. Ver `carregar_ou_criar_posicao`.
        let (mut afiliado_posicao, _criada) = carregar_ou_criar_posicao(
            &ctx.remaining_accounts[base + 3],
            &afiliado,
            &ctx.accounts.vault,
            afiliado_dom.amount,
            &ctx.accounts.payer.to_account_info(),
            ctx.accounts.system_program.key(),
        )?;
        let v = &ctx.accounts.vault;
        // ⚠️ N7: `ganho_indicado × bps / 1e4`, SEM o `perf_fee`. Ver o bloco 3.
        let comissao = u64::try_from(
            (ganho_indicado as u128)
                .checked_mul(bps as u128)
                .ok_or(DomError::MathOverflow)?
                / BPS_DEN,
        )
        .map_err(|_| error!(DomError::MathOverflow))?;
        comissoes_total = comissoes_total
            .checked_add(comissao)
            .ok_or(DomError::MathOverflow)?;
        require!(comissoes_total <= parcela_socios, DomError::MathOverflow);

        // -------------------------------------------------------------------
        // ITEM 7 · O MÍNIMO DE 1 USDC, E A RESERVA PRÓPRIA DO AFILIADO
        // (ordem do dono, 13/10 13:10)
        // -------------------------------------------------------------------
        // Comissão abaixo de `MIN_COMISSAO_CUNHAGEM_USDC` **não cunha**: ela
        // acumula na reserva do próprio afiliado e é paga INTEIRA quando a soma
        // alcança o mínimo.
        //
        // ⚠️ **E nada do que não cunhou vai para a mesa.** O débito da mesa
        // (`comissoes_total`) continua sendo a comissão **DEVIDA** deste ciclo,
        // cunhada ou não — é a linha logo acima, e ela não muda. O USDC
        // correspondente fica no caixa sem cota emitida contra ele até ser pago,
        // e por isso a reserva é a **dívida** que diz de quem ele é.
        //
        // ⓘ E a conta fecha no pagamento: na hora de pagar, cunha-se
        // `comissão deste ciclo + reserva`, e o lastro são os dois USDC — o
        // debitado agora e o que ficou no caixa desde o ciclo em que acumulou.
        // Debitar a mesa de novo pela reserva seria cobrar duas vezes.
        //
        // ⚠️ Por que a devnet mediu isto como "A3 recebeu 0,499999": com o
        // binário `8e7b5bf` a comissão cunhava sempre. O devnet perguntou qual
        // leitura valia (§8.3 da MEDICAO-ITEM-7-13-10) e a ordem respondeu: não
        // cunha.
        let (acumulado_antes, bump_reserva) =
            reserva_do_afiliado(&ctx.remaining_accounts[base + 4], &afiliado)?;
        let devido = comissao
            .checked_add(acumulado_antes)
            .ok_or(DomError::MathOverflow)?;
        let paga_agora = devido >= MIN_COMISSAO_CUNHAGEM_USDC;

        // ⚠️ `comissao_usdc` do evento passa a ser o DEVIDO quando paga, porque é
        // ele que foi precificado em cota — a invariante
        // `cotas_comissao == shares_from_usdc(comissao_usdc, nav_fechamento)`
        // continua verdadeira no evento. O que veio do ciclo e o que veio da
        // reserva saem separados no `ComissaoDaReservaPaga`. Com reserva zero —
        // o caso de hoje — `devido == comissao` e nada muda.
        let cotas_comissao = if paga_agora {
            shares_from_usdc(devido, nav_fechamento)?
        } else {
            0
        };
        // ⚠️ SEM `cunhar_com_acerto` quando não paga, e não com `cotas_base = 0`:
        // aquela função aplica o ACERTO pendente da carteira na cunhagem —
        // crédito cunharia cota sem comissão, e débito levantaria
        // `AcertoPendente` num afiliado que não ia receber nada. Quem não recebe
        // não acerta, e o acerto dele continua pendente para a próxima vez.
        let cotas = if paga_agora {
            cunhar_com_acerto(
                v,
                &mut afiliado_posicao,
                &afiliado,
                afiliado_dom.amount,
                &ctx.remaining_accounts[base + 2],
                cotas_comissao,
                nav_fechamento,
                &ctx.accounts.dom_mint.to_account_info(),
                &ctx.accounts.dom_token_program,
                vault_seeds,
            )?
        } else {
            0
        };
        comissoes_cunhadas = comissoes_cunhadas
            .checked_add(if paga_agora { devido } else { 0 })
            .ok_or(DomError::MathOverflow)?;
        afiliado_posicao.exit(&crate::ID)?;

        // A reserva só é escrita quando o estado dela MUDA, e só nasce quando há
        // o que guardar: comissão que já alcança o mínimo é paga na hora e não
        // paga rent de conta nenhuma.
        if paga_agora {
            if acumulado_antes > 0 {
                gravar_reserva(
                    &ctx.remaining_accounts[base + 4],
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
                    do_ciclo_micro: comissao,
                    da_reserva_micro: acumulado_antes,
                    total_micro: devido,
                    timestamp: now,
                });
            }
        } else if devido > acumulado_antes {
            // ⚠️ `devido > acumulado_antes`, e não só `else`. Com comissão ZERO —
            // indicado com ganho zero (carteira que resgatou tudo, lista velha)
            // ou truncamento (`ganho × bps < 10.000`) — o `else` puro criava a
            // conta da reserva para guardar um ZERO e queimava o rent
            // (medido: 899.160 lamports na mainnet) do bolso da mesa, sem
            // `close` que o devolva. E emitia um evento de acúmulo sem acúmulo.
            //
            // Medido pela revisão adversária de 13/10: `ComissaoAcumulada`
            // (ganho 0, comissão 0, acumulado 0) com a conta criada, 49 bytes.
            gravar_reserva(
                &ctx.remaining_accounts[base + 4],
                &afiliado,
                bump_reserva,
                devido,
                &ctx.accounts.payer.to_account_info(),
                ctx.accounts.system_program.key(),
            )?;
            // DECISOES 21:00: o que entrou na reserva neste ciclo é dívida nova.
            reservas_guardadas = reservas_guardadas
                .checked_add(devido - acumulado_antes)
                .ok_or(DomError::MathOverflow)?;
            emit!(ComissaoAcumulada {
                indicado,
                afiliado,
                bps,
                ganho_indicado,
                comissao_usdc: comissao,
                acumulado_micro: devido,
                minimo_micro: MIN_COMISSAO_CUNHAGEM_USDC,
                timestamp: now,
            });
        }

        if !paga_agora {
            // sem cunhagem não sai `ComissaoDeAfiliado`: dois eventos para o
            // mesmo fato enganariam quem reconcilia a soma das comissões.
            continue;
        }
        emit!(ComissaoDeAfiliado {
            indicado,
            afiliado,
            bps,
            ganho_indicado,
            comissao_usdc: devido,
            cotas,
            cotas_comissao,
            /* i64 porque o acerto pode ser DÉBITO: a cunhagem sai menor que a comissão. */
            cotas_acerto: (cotas as i64)
                .checked_sub(cotas_comissao as i64)
                .ok_or(DomError::MathOverflow)?,
            timestamp: now,
        });
    }

    // 4 · o resto da mesa para os socios, com o acerto de cada um compensado
    // na cunhagem (J7 sem assinatura): cunha `cotas_por_socio + credito − debito`.
    // Se o debito passar disso — socio segurando mais de um terco do fundo —
    // ele assina `acertar` antes e o fechamento e' repetido.
    // -----------------------------------------------------------------------
    // ⚠️ A CUSTODIA SAI DA PARTE DA MESA — senao o mesmo P e distribuido DUAS VEZES
    // -----------------------------------------------------------------------
    // Decisao aprovada (DECISOES 13/10 16:55, item 1, textual): *"separa em
    // custodia o orcamento das comissoes **(da parte da mesa)**"*; e a decisao 7
    // manda a sobra **voltar a mesa** — o residual so volta para quem foi debitado.
    // O canone da casa (`dom-regras-do-dinheiro`): *"comissao debitada da parte da
    // mesa; soma das comissoes <= parte da mesa"*.
    //
    // ⚠️ A PRIMEIRA VERSAO DESTE BLOCO NAO DEBITAVA NINGUEM, e a conta nao fechava.
    // Medido pela revisao adversaria de 13/10, com P = 1.000 USDC e perf_fee 5.000:
    //
    //   janela do cotista .... 500.000.002
    //   mesa, ja cunhada ..... 500.000.000   <- a mesa INTEIRA, sem desconto
    //   afiliados, pelo retrato  240.000.000
    //   ------------------------------------
    //   reivindicado ......... 1.240.000.002  contra 1.000.000.000 de caixa novo
    //
    // Os 240 USDC saiam do capital dos cotistas. E o teste 9 do bloco anterior
    // afirmava `supply_sem == supply_com` com a mensagem "o orcamento NAO cunha
    // cota" — teste verde provando o erro: aquela igualdade ERA a prova de que a
    // mesa nao tinha sido debitada.
    //
    // ⚠️ E a trava de teto, que tambem faltava: sem ela um voto absurdo passava e o
    // piso ia a zero em silencio pelo `saturating_sub`.
    let devido_da_mesa = comissoes_total
        .checked_add(orcamento_comissoes_micro)
        .ok_or(DomError::MathOverflow)?;
    require!(
        devido_da_mesa <= parcela_socios,
        DomError::CustodiaAcimaDaParteDaMesa
    );
    let mesa_dos_socios = parcela_socios - devido_da_mesa;
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
        // ⓘ O RETORNO É O QUE DE FATO FOI CUNHADO (o acerto do sócio entra netado),
        // e ele NÃO é publicado: o `LucroDistribuido` leva `cotas_por_socio`, que é
        // o PRETENDIDO. Era o `cunhadas_total` que somava o real, e ninguém o lia —
        // saiu por ordem do dono (14/10 23:00 item 3). Quem precisa do netado por
        // sócio lê a posição dele. Limite declarado, não esquecido.
        let _cunhadas = cunhar_com_acerto(
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
        posicao.exit(&crate::ID)?;
    }
    let livros = [
        ctx.accounts.socio0_ledger.to_account_info(),
        ctx.accounts.socio1_ledger.to_account_info(),
        ctx.accounts.socio2_ledger.to_account_info(),
    ];
    for (i, info) in livros.iter().enumerate() {
        somar_no_livro(
            info,
            &socios[i],
            cotas_por_socio,
            &ctx.accounts.payer.to_account_info(),
            ctx.accounts.system_program.key(),
        )?;
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

    // -----------------------------------------------------------------------
    // FASE A · O RETRATO DO CICLO
    // -----------------------------------------------------------------------
    // ⚠️ A POSICAO DESTE BLOCO E' A REGRA, NAO ESTILO. Ele vem:
    //
    //   DEPOIS dos indices (eles tomam o valor do fechamento la' atras) — senao o
    //          retrato fotografaria o ciclo PASSADO e a Fase B pagaria o ganho
    //          errado;
    //   DEPOIS do laco dos afiliados — senao `comissoes_total` ainda e' zero e o
    //          retrato registraria "zero comissao" num ciclo que pagou comissao;
    //   ANTES do piso — e ele le a divida daqui.
    //
    // A primeira versao gravava antes do laco e registrava 0 comissao em ciclos que
    // pagaram 20 linhas. Achado da revisao adversaria de 13/10.
    {
        let v = &ctx.accounts.vault;
        let indice_ciclo_anterior = v.indice_ciclo_anterior;
        let indice_ciclo = v.indice_ciclo;
        let indice_diluicao = v.indice_diluicao;
        let r = &mut ctx.accounts.retrato;
        r.ciclo = ciclo;
        r.indice_ciclo_anterior = indice_ciclo_anterior;
        r.indice_ciclo = indice_ciclo;
        r.indice_diluicao = indice_diluicao;
        r.nav_fechamento = nav_fechamento;
        r.bump = ctx.bumps.retrato;
        // ⚠️ DECISOES 13/10 21:00 · o total das reservas em aberto ATRAVESSA os
        // ciclos: herda do retrato anterior e aplica o delta deste fechamento.
        //
        // ⓘ No primeiro ciclo com retrato o herdado é ZERO, e isso é uma hipótese
        // declarada: reservas criadas ANTES do primeiro retrato não entram na conta.
        // Em produção não há nenhuma (a reserva nasceu no item 7, que não subiu).
        let herdado = herdar_reservas(&ctx.accounts.retrato_anterior, ciclo)?;
        r.reservas_em_aberto_micro = herdado
            .checked_add(reservas_guardadas)
            .ok_or(DomError::MathOverflow)?
            .saturating_sub(reservas_pagas);
        if orcamento_comissoes_micro > 0 {
            // O caminho da CUSTODIA: a Fase B paga, e ate la' ha divida.
            r.orcamento_micro = orcamento_comissoes_micro;
            r.gasto_micro = 0;
            r.linhas_votadas = linhas_votadas;
            r.linhas_pagas = 0;
            r.aberto = true;
        } else {
            // ⚠️ O caminho ANTIGO: a comissao JA' foi paga aqui dentro. O retrato
            // registra o que o ciclo pagou — votado == gasto, divida zero — e nasce
            // FECHADO.
            //
            // Nasce fechado por uma razao dura: com `aberto = true` aqui, o ciclo
            // seguinte recusaria com `RetratoAnteriorAberto` e NINGUEM poderia
            // fechar este retrato (a Fase B e o `encerrar_ciclo` nao existem). O
            // cofre travaria.
            r.orcamento_micro = comissoes_total;
            r.gasto_micro = comissoes_total;
            r.linhas_votadas = afiliados.len() as u16;
            r.linhas_pagas = afiliados.len() as u16;
            r.aberto = false;
        }
        emit!(crate::events::RetratoAberto {
            ciclo,
            indice_ciclo_anterior,
            indice_ciclo,
            nav_fechamento,
            orcamento_micro: r.orcamento_micro,
            linhas_votadas: r.linhas_votadas,
            aberto: r.aberto,
            timestamp: now,
        });
    }

    // 6 · o ciclo vira
    let vault = &mut ctx.accounts.vault;
    // ⚠️ O EXCEDENTE FICA NA GAVETA, E VIRA O NOVO PONTO ZERO.
    //
    // A transferencia acima moveu `p` (o votado); a gaveta guarda `saldo - p` —
    // o ponto zero antigo mais a poeira que chegou. Gravar isso em
    // `gaveta_saldo_visto` com `p_ciclo = 0` mantem `z = visto - p_ciclo` exato, e
    // a invariante da conservacao continua valendo no ciclo seguinte.
    //
    // ⚠️ A ALTERNATIVA ERA ZERAR OS DOIS, e ela e PIOR: o excedente viraria P do
    // proximo ciclo na primeira absorcao, ou seja, poeira de terceiro entraria
    // silenciosamente no lucro a distribuir. Assim ela fica NOMEADA como saldo nao
    // reconhecido, e se a mesa quiser distribui-la basta votar um numero que a
    // inclua — decisao explicita, no lugar de um efeito colateral.
    vault.gaveta_saldo_visto = saldo.saturating_sub(p);
    vault.p_ciclo = 0;
    vault.delta_lucro_por_cota = 0; // legado: a janela passa a ser pelo indice (J5)
                                    // -----------------------------------------------------------------------
                                    // ITEM 7 · O TETO DA JANELA INCLUI AS COMISSÕES QUE VIRARAM COTA
                                    // -----------------------------------------------------------------------
                                    // O escopo (§7.1, ponto 2) pede `parcela_cotistas + Σ comissões`. Aqui entra
                                    // `comissoes_cunhadas`, e **não** `comissoes_total`, e a diferença é
                                    // declarada: a comissão que ficou na reserva não é reivindicável por
                                    // ninguém — somar ao teto um valor sem titular afrouxa a segunda cinta do
                                    // `sacar_lucro` sem beneficiário, e o teto é cinta. Quando a reserva for
                                    // paga, ela entra no `comissoes_cunhadas` DAQUELE fechamento, e o teto é
                                    // reaberto a cada fechamento: não há contagem em dobro.
                                    //
                                    // ⚠️ E o que isto NÃO faz: o afiliado ainda não tem caminho para realizar a
                                    // comissão em USDC (é o 7.6, fora deste bloco). Enquanto ele não existir,
                                    // este acréscimo é preparação — nenhum cotista alcança mais dinheiro por
                                    // causa dele, porque o direito de cada carteira continua sendo o `ganho`
                                    // dela pelo índice; o teto é só a cinta global.
                                    //
                                    // ⚠️ **E O TETO NUNCA PASSA DE `p`.** `comissoes_cunhadas` inclui reserva de
                                    // ciclos ANTERIORES, e a trava `comissoes_total <= parcela_socios` limita só
                                    // o devido DESTE ciclo. Sem o `min`, onze afiliados com 0,999999 acumulado
                                    // cada, num fechamento de 2 USDC, abririam uma janela de 12 USDC sobre 2
                                    // USDC de caixa novo — folga sem titular, que afrouxa a segunda cinta do
                                    // `sacar_lucro` e publica um `nao_sacado` inflado por dinheiro que ninguém
                                    // podia sacar. Achado da revisão adversária de 13/10; a desigualdade sai das
                                    // duas linhas, não de ensaio.
                                    // -------------------------------------------------------------------------
                                    // ⚠️ A CUSTODIA **NAO** ENTRA NO TETO — E E' REQUISITO DA FASE B
                                    // -------------------------------------------------------------------------
                                    // A revisao adversaria apontou, com razao, que as comissoes que a Fase B cunhar
                                    // ficariam fora do teto que o item 7 manda incluir. Eu tentei consertar somando
                                    // `orcamento_comissoes_micro` aqui, e estava ERRADO: o teste 9 mediu
                                    //
                                    //   mesa 259.999.995 + custodia 240.000.000 + janela 740.000.002
                                    //     = 1.239.999.997  contra  P = 1.000.000.000
                                    //
                                    // ou seja, a custodia contada duas vezes — a mesa ja' abriu mao dela.
                                    //
                                    // E a regra certa esta' escrita no proprio comentario acima, para a reserva: o
                                    // teto so conta o que TEM TITULAR. Enquanto a Fase B nao cunhar, ninguem pode
                                    // sacar a custodia; somar um valor sem titular afrouxa a cinta.
                                    //
                                    // ⚠️ REQUISITO DA FASE B, para quem a escrever: ao cunhar a comissao de uma
                                    // linha, a Fase B tem de ACRESCENTAR aquele valor a
                                    // `vault.lucro_sacavel_restante` (com o mesmo `.min(p)` do ciclo), porque o
                                    // afiliado passa a poder sacar naquele instante. E' o mesmo tratamento que a
                                    // reserva recebe quando e' paga.
    vault.lucro_sacavel_restante = parcela_cotistas
        .checked_add(comissoes_cunhadas)
        .ok_or(DomError::MathOverflow)?
        .min(p);
    vault.ultima_distribuicao_ts = now;
    // ⚠️ UPGRADE N: as duas contas que este bloco fazia — o caixa com o P e o
    // supply depois da cunhagem — eram insumo da FORMULA do piso. Com a marca
    // elas nao servem mais, e eu as removi em vez de as deixar calculadas e
    // ignoradas: variavel viva que ninguem le e' a proxima a ser lida por
    // engano.
    {
        // -------------------------------------------------------------------
        // ⚠️ UPGRADE N · O PISO RESETA NO FECHAMENTO, PARA O NAV
        //
        // A gaveta foi varrida alguns passos acima: o motivo do piso saiu com
        // ela. Entao o piso volta a ser o NAV — e o ciclo seguinte recomeca a
        // marcar do zero, na primeira realizacao de lucro.
        //
        // ⚠️ NAV POS-FECHAMENTO, NAO ZERO. Os dois dao o mesmo PRECO, porque
        // `deposit.rs` cobra `max(nav, nav_piso)` e `max(nav, 0) == nav`. Mas
        // `nav_piso` e' um campo PUBLICADO: o laudo o grava, a vista o serve e
        // a tela o mostra. `nav_piso_micro: "0"` numa tela se le como erro de
        // leitura, nao como "nao ha piso". O campo tem de DIZER a verdade, e a
        // verdade e' "o piso e' o NAV de agora".
        //
        // E contra piso velho os dois protegem igual: o que protege e' o reset
        // ACONTECER, nao o valor dele.
        //
        // ─── ⚠️ O NAV *JA' DILUIDO*, E EU ESTAVA ERRADO AQUI ────────────────
        //
        // A primeira versao gravava `vault.nav` tal como estava, com este
        // argumento: "o `nav` e' do oraculo e nao conhece a cunhagem de agora;
        // gravar um NAV recalculado a mao seria o contrato opinando sobre
        // marcacao, que e' o que a D-F2-43 §3 proibe".
        //
        // A DEVNET DERRUBOU ISSO (teste 8 do UPGRADE-N-ENSAIO-DEVNET): com o
        // NAV a 6,70 antes do fechamento e 6,63 depois da diluicao, o piso
        // ficava em 6,70 — e, como a marca NAO desce na publicacao seguinte
        // (ela so e' regravada quando entra lucro novo), o aporte saia 1,06%
        // ACIMA do NAV ate' a realizacao seguinte. Que e' o defeito que o item
        // 1 existe para matar.
        //
        // ⚠️ E O MEU ARGUMENTO NAO SE SUSTENTA: diluicao por cunhagem interna
        // nao e' marcacao, e' DIVISAO — e o que a D-F2-43 §3 proibe e' o
        // contrato INVENTAR valor de mercado, nao dividir patrimonio por
        // supply. O piso tem de ser o preco POS-fechamento.
        //
        // ⓘ E por que `vault.nav` continua intocado: o NAV publicado e' do
        // oraculo, e o publicador o recalcula na proxima corrida a partir das
        // pernas. Quem tem de dizer a verdade AGORA e' o `nav_piso`, porque e'
        // ele que precifica o aporte no intervalo.
        // -------------------------------------------------------------------
        // ⓘ `supply_depois` SAIU, e com ele a conta que o usava. Até 14/10 o piso
        // era `(nav × supply − divida × SCALE) / supply_depois`: uma divisão pelo
        // supply DEPOIS da cunhagem, com a dívida fora. A regra de 17:50 troca
        // tudo isso pelo `nav_fechamento`, que já é o preço pós-diluição — e é o
        // mesmo número pelo qual a janela e o acerto convertem.
        //
        // ⚠️ E a diluição NÃO saiu: ela está DENTRO do `nav_fechamento`
        // (`nav − mesa_por_cota`), não fora. O que saiu foi o segundo desconto,
        // não o primeiro.
        //
        // ⚠️ **E NÃO PROCURE AQUI A DEFESA DA FÓRMULA ANTIGA**: até a revisão
        // adversária do `2df5f81` este bloco ainda argumentava, em 30 linhas, a
        // favor do `nav × supply_antes / supply_depois` que a ordem de 17:50
        // revogou, e afirmava que "sem cunhagem devolve o próprio `nav`" — falso
        // desde que a custódia pode absorver a parte da mesa inteira: ali não se
        // cunha nada e o piso é `nav − mesa_por_cota`, ABAIXO de `nav`.
        // Os números de linha que o parágrafo citava (`:190`, `:532`) também já
        // apontavam para outro código. Ficou o que é verdade.
        // ─────────────────────────────────────────────────────────────────
        // ⚠️ DECISOES 14/10 17:50 · O PISO E O `nav_fechamento`, E O PROGRAMA
        //    NAO DESCONTA MAIS A DIVIDA
        // ─────────────────────────────────────────────────────────────────
        // Ordem do dono, textual: *"NAV PUBLICADO E LIQUIDO: o backend tira a
        // reserva e a custodia do patrimonio (14:50). Por isso o programa NAO
        // desconta a divida herdada de novo: piso = nav_fechamento (div_ceil),
        // FIXADO NA FASE A; os lotes nao regravam o piso. Revoga a formula
        // piso = nav_fechamento − herdado/supply de 16:00 (ela supunha NAV
        // BRUTO e descontava duas vezes)."*
        //
        // 🔴 **O DESCONTO DUPLO FOI MEDIDO, dos dois lados.** O devnet mediu
        // **−208, −262, −133 e −296** micro por cota com o binario anterior
        // contra o oraculo de NAV liquido; e eu medi, pelo lado do backend, o
        // retrato do ciclo 4 da devnet (reservas 512.499 micro, supply
        // 3.055.131.122) dar **167 micro por cota** a mais se as duas pontas
        // descontarem. ⚠️ Ha DOIS conjuntos do devnet (±1 micro, pelo
        // `supply_depois`): a procedencia esta em `docs/UPGRADE-N.md`.
        //
        // ─── por que a conta FECHA assim, e nao e' "deixar de descontar" ────
        // O `nav` que entra aqui ja' vem LIQUIDO do oraculo: a reserva e a
        // custodia ABERTA sairam do patrimonio antes da divisao
        // (`lib/passivo-do-cofre.ts` no backend-nav). O `nav_fechamento` e'
        // `nav − mesa_por_cota` — o preco POS-diluicao da mesa, que e'
        // exatamente o preco pelo qual a janela e o acerto convertem. Descontar
        // a divida DE NOVO aqui tiraria do piso um dinheiro que o NAV ja' nao
        // conta.
        //
        // ⓘ O `div_ceil` da ordem ESTA CUMPRIDO: `nav − ⌊a/S⌋ == ⌈(nav·S − a)/S⌉`
        // (medido em inteiros, 300.000 sorteios, 0 falhas). ⚠️ E NAO use
        // `⌈nav × supply_antes / supply_depois⌉` como referencia: ela e a
        // conservacao BRUTA, ignora a divida criada no fechamento, e com 240 USDC
        // de custodia da 23.421 micro de diferenca. A RETRATACAO inteira (eu levei
        // ao dono um "1 micro" que era o `ceil` da referencia) esta no corpo de
        // `tests/piso_de_entrada.rs` e em `docs/UPGRADE-N.md`.
        //
        // ⚠️ O QUE ISTO REVOGA e a formula de **16:00** (piso descendo
        // `herdado × SCALE / supply_depois`), que valia para NAV BRUTO.
        //
        // ⓘ FIXADO NA FASE A = os LOTES nao regravam (`pagar_comissoes` nao toca
        // `nav_piso`; o teste `fase_b_os_lotes_nao_regravam_o_piso` cobra).
        // ✅ E o `publish_nav` tambem nao: com o retrato ABERTO ele nao marca o
        // piso (o (3a), `publish_nav.rs` `piso_livre`). Antes regravava dentro da
        // janela da Fase B, e isso ficou declarado aqui ate a ordem de 14/10 23:00;
        // a minha primeira versao desta regra APAGOU a declaracao sem o GO.
        //
        // ⚠️ E POR QUE ESTE BLOCO E CURTO: comentario aqui CUSTA CU. Medido —
        // 25 linhas de comentario neste arquivo mudaram 21 bytes do `.so` (sao os
        // `line!()` que o Anchor compila dentro de cada `require!`) e levaram o
        // `deposit_especial` com 3 socios + 3 afiliados de dentro do teto a
        // **176.583 CU (12% de margem)**, reprovando o portao de CU da casa. O
        // texto longo mora no teste e no doc; aqui fica o que decide.
        let piso_novo = nav_fechamento;
        let piso_anterior = vault.nav_piso;
        vault.nav_piso = piso_novo;
        emit!(crate::events::PisoResetado {
            piso_anterior,
            piso_novo,
            timestamp: now,
        });
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

/// ─── ITEM 7.2 · A POSIÇÃO DO AFILIADO, CRIADA AQUI DENTRO ───────────────────
///
/// Ordem do dono (12/10): *"posição do afiliado criada por CPI dentro do
/// `deposit_especial`, com o rent pago pela mesa"*. Decisão registrada antes
/// (DECISOES 12/10): **`create_account` + init manual, rent pela mesa, sem
/// `init_if_needed`**.
///
/// ─── por que CPI e não `init_if_needed` ─────────────────────────────────────
/// `init_if_needed` é **constraint de campo declarado** e não alcança
/// `remaining_accounts` — e as posições dos afiliados chegam por ali, porque a
/// lista é variável. Mover para a struct não é opção: foi medido em 09/10
/// (`docs/ITEM-7-CUSTO-DE-CONTAS-09-10.md`) que o `try_accounts` de uma struct com
/// os pares de afiliado declarados estoura a pilha. O custo de contas do CPI é
/// **ZERO**: `payer` e `system_program` já estão na struct desta instrução.
///
/// ⚠️ TRÊS COISAS QUE ESTA FUNÇÃO FAZ À MÃO, e cada uma já custou caro em algum
/// lugar desta casa:
///
///  1 **o discriminador.** O Anchor só o grava dentro do `init`. Conta sem
///    discriminador é conta que o próximo `Account<T>` recusa com
///    `AccountDiscriminatorNotFound` — três passos depois da causa. Mesmo gesto do
///    `update_whitelist.rs:164`.
///
///  2 **todos os campos, sempre.** Não há estado parcial herdado: era a garantia
///    que o `init_if_needed` dava, mantida à mão (o comentário do
///    `update_whitelist.rs:161` diz isso com estas palavras).
///
///  3 ⚠️ **`indice_entrada` NÃO É ZERO: é o índice de AGORA.** Zero daria ao
///    afiliado direito ao P que acabou de ser varrido — o mesmo cuidado da
///    cunhagem da mesa. Neste ponto da instrução o ciclo já virou
///    (`vault.indice_ciclo = vault.indice_p`, acima), então os dois são iguais e a
///    posição **nasce acertada**: `indice_diluicao_visto` e `indice_p_acertado` no
///    estado do cofre, nada pago e nada devido até aqui. É a mesma regra do
///    `handle_abrir_posicao` (`gaveta.rs:140-151`), com a diferença de que ali a
///    carteira pode ter cota de antes do ciclo e aqui o fechamento já passou.
///
/// ⚠️ **O rent sai da MESA** (`payer`, que no fechamento é o vault do Squads):
/// `0,001102360 SOL` por posição criada, medido em 09/10. Quem paga está na
/// struct e assina a instrução — não há caminho para outro pagador.
///
/// ⚠️ **E O CU DESTE CPI NÃO ESTÁ MEDIDO.** É o único número que falta para fechar
/// o item, e ele sai do ensaio na devnet — não daqui.
pub(crate) fn carregar_ou_criar_posicao<'info>(
    info: &'info AccountInfo<'info>,
    owner: &Pubkey,
    vault: &Vault,
    // O saldo de cota do afiliado AGORA. Com cota > 0 esta funcao RECUSA criar —
    // ver a nota "o confisco que esta funcao nao comete".
    cotas_do_afiliado: u64,
    payer: &AccountInfo<'info>,
    system_program_id: Pubkey,
) -> Result<(Account<'info, PosicaoDoCotista>, bool)> {
    let criada = info.data_is_empty();
    if criada {
        // A busca canônica fica SÓ na criação: é ela que garante uma posição por
        // carteira (ver `conferir_pda_pelo_bump_gravado`).
        let (esperada, bump) =
            Pubkey::find_program_address(&[POSICAO_SEED, owner.as_ref()], &crate::ID);
        require_keys_eq!(info.key(), esperada, DomError::PosicaoDoCotistaAusente);
        require!(info.is_writable, DomError::PosicaoDoCotistaAusente);
        // ---------------------------------------------------------------------
        // ⚠️ O CONFISCO QUE ESTA FUNÇÃO NÃO COMETE
        // ---------------------------------------------------------------------
        // A posição nasce com `indice_entrada = indice_ciclo`, e neste ponto o
        // ciclo JÁ VIROU (`indice_ciclo = indice_p`, bloco 6a acima). Para quem
        // **já tem cota**, isso apaga a janela do ciclo que acabou de fechar:
        // `sacar_lucro.rs:189` faz `ganho(cotas, entrada, indice_ciclo_anterior,
        // indice_ciclo)` e, com `entrada == indice_ciclo`, a referência não passa
        // do piso — o direito dele vira ZERO.
        //
        // Medido: 1.000 cotas num ciclo de 1.000 USDC sobre 96.000 cotas valem
        // 10,416666 USDC de janela se a posição existisse, e 0,000000 com a
        // posição criada aqui.
        //
        // ⚠️ E ESTA AMEAÇA ESTÁ NOMEADA NO PRÓPRIO CÓDIGO DESTA CASA:
        // `gaveta.rs:118-124` — *"abrir a posicao de um holder DEPOIS de um P
        // absorvido o faria perder esse P (ameaca 'abrir posicao alheia depois de
        // P')"*. Lá a defesa é usar `indice_ciclo` com cota > 0, porque lá ele é o
        // piso do ciclo **aberto**; aqui ele é o piso do ciclo **recém-virado**, e
        // a mesma linha produz o efeito oposto.
        //
        // Então: **criação só para carteira SEM cota**, que é o único caso em que
        // "nasce acertada ao índice de agora" não tira nada de ninguém. Afiliado
        // com cota e sem posição é anomalia de lista — a mesa roda `abrir_posicao`
        // ANTES do fechamento, ainda no ciclo aberto, e a janela dele fica
        // preservada. Recusar alto é melhor que confiscar calado.
        //
        // ⚠️ E isso fecha um segundo buraco, de ordem: com a lista
        // `[(indicado A, afiliado X), (indicado X, afiliado B)]` e X sem posição, a
        // primeira linha criaria a posição de X e a segunda calcularia o ganho de X
        // como ZERO — a comissão de B viraria zero **e a instrução passaria**. Com
        // a recusa, X sem cota tem ganho zero de qualquer forma (sem cota não há
        // ganho) e X com cota recusa na primeira linha, alto.
        // ---------------------------------------------------------------------
        require!(cotas_do_afiliado == 0, DomError::PosicaoDoCotistaAusente);
        let espaco = 8 + PosicaoDoCotista::INIT_SPACE;
        let seeds: &[&[u8]] = &[POSICAO_SEED, owner.as_ref(), &[bump]];
        // ---------------------------------------------------------------------
        // ⚠️ NEGAÇÃO DE SERVIÇO POR 1 LAMPORT — MEDIDA, NÃO SUPOSTA
        // ---------------------------------------------------------------------
        // `create_account` RECUSA uma conta que já tem lamports:
        //
        //   "Create Account: account Address { address: Gzq… } already in use"
        //   Program 1111…1111 failed: custom program error: 0x0
        //
        // E o endereço de uma PDA é público e derivável por qualquer um. Então
        // **qualquer pessoa na rede manda 1 lamport para a PDA da posição de um
        // afiliado e derruba o fechamento do ciclo inteiro** — de novo, e de novo,
        // por fração de centavo por tentativa.
        //
        // ⚠️ ESTA CASA JÁ SOFREU ESTE ATAQUE, em outro lugar: em 05/10 chegaram
        // QUATRO poeiras na gaveta, de dois endereços vanity construídos sobre a
        // Operacional do fundo — e foi por isso que a trava de igualdade do `p`
        // virou piso no Upgrade L (`:262`). O vetor é conhecido nesta casa.
        //
        // Medido com `cargo test n72_poeira`: com o `create_account` puro, 1
        // lamport na PDA derrubava a instrução INTEIRA. O caminho abaixo resiste:
        // completa o rent que falta, ALOCA o espaço e ATRIBUI o dono — os três
        // passos que o `create_account` faz junto, e que separados não se importam
        // com o saldo que já estava lá.
        // ---------------------------------------------------------------------
        abrir_conta_do_programa(info, seeds, espaco, payer, system_program_id)?;
        // ⚠️ `try_serialize` E NÃO OFFSETS À MÃO. A primeira versão escrevia
        // `data[8..40]`, `data[40..56]`, … — e offset à mão não tem amarra com a
        // struct: trocar dois `u128` de lugar no `state.rs` mantém os 81 bytes e
        // passa a escrever nos campos errados **em silêncio**. O `try_serialize`
        // grava o discriminador e a ordem da PRÓPRIA struct.
        let nova = PosicaoDoCotista {
            owner: *owner,
            indice_entrada: vault.indice_ciclo,
            indice_diluicao_visto: vault.indice_diluicao,
            indice_p_acertado: vault.indice_ciclo,
            bump,
        };
        let mut data = info.try_borrow_mut_data()?;
        require!(data.len() == espaco, DomError::PosicaoDoCotistaAusente);
        nova.try_serialize(&mut &mut data[..])?;
    } else {
        require!(info.is_writable, DomError::PosicaoDoCotistaAusente);
    }

    let posicao: Account<PosicaoDoCotista> = Account::try_from(info)?;
    if !criada {
        conferir_pda_pelo_bump_gravado(
            info.key,
            [POSICAO_SEED, owner.as_ref()],
            posicao.bump,
            DomError::PosicaoDoCotistaAusente,
        )?;
    }
    require_keys_eq!(posicao.owner, *owner, DomError::PosicaoDoCotistaAusente);
    Ok((posicao, criada))
}

/// A PDA de uma conta QUE JÁ EXISTE, conferida pelo bump GRAVADO nela
/// (DECISOES 19/10 17:05).
///
/// O `find_program_address` tenta do bump 255 para baixo, 1.500 CU por tentativa,
/// e era essa busca que fazia o CU do fechamento variar (160.030 a 185.530 no
/// mesmo `.so`, conforme as carteiras). O `create_program_address` com o bump
/// gravado é uma tentativa só, sempre.
///
/// ⚠️ **SÓ para conta deste programa, lida com dono e discriminador ANTES.** O
/// programa só cria pela busca canônica, então o bump gravado É o canônico. Bump
/// vindo de fora (argumento, cliente) não prova isso: a mesma semente tem outro
/// endereço válido em cada bump abaixo do canônico, e o `init` do retrato
/// deixaria de impedir o mesmo ciclo fechado duas vezes. Conta que NASCE continua
/// na busca canônica.
pub(crate) fn conferir_pda_pelo_bump_gravado(
    chave: &Pubkey,
    sementes: [&[u8]; 2],
    bump: u8,
    erro: DomError,
) -> Result<()> {
    let bump = [bump];
    let esperada = Pubkey::create_program_address(&[sementes[0], sementes[1], &bump], &crate::ID)
        .map_err(|_| error!(erro))?;
    require_keys_eq!(*chave, esperada, erro);
    Ok(())
}

/// Soma as cotas do fechamento no livro de um sócio (`FeeShareLedger`): abre o
/// livro na primeira vez, pela busca canônica; depois, confere pelo bump gravado.
/// Era um `init_if_needed`, e o Anchor faz a busca canônica em TODA chamada dele,
/// mesmo com a conta já aberta: 3 buscas por fechamento, uma por sócio.
fn somar_no_livro<'info>(
    info: &AccountInfo<'info>,
    socio: &Pubkey,
    cotas: u64,
    payer: &AccountInfo<'info>,
    system_program_id: Pubkey,
) -> Result<()> {
    // Gravável é o `#[account(mut)]` da struct que exige (ConstraintMut).
    let espaco = 8 + FeeShareLedger::INIT_SPACE;
    let mut livro = if info.data_is_empty() {
        let (esperada, bump) =
            Pubkey::find_program_address(&[FEE_SHARE_SEED, socio.as_ref()], &crate::ID);
        require_keys_eq!(info.key(), esperada, DomError::LivroDoSocioInvalido);
        let seeds: &[&[u8]] = &[FEE_SHARE_SEED, socio.as_ref(), &[bump]];
        abrir_conta_do_programa(info, seeds, espaco, payer, system_program_id)?;
        FeeShareLedger {
            socio: *socio,
            shares: 0,
            bump,
        }
    } else {
        // Dono, depois discriminador (`try_deserialize`), e só então o bump
        // gravado vale como prova do endereço.
        require_keys_eq!(*info.owner, crate::ID, DomError::LivroDoSocioInvalido);
        let dados = info.try_borrow_data()?;
        let livro = FeeShareLedger::try_deserialize(&mut &dados[..])
            .map_err(|_| error!(DomError::LivroDoSocioInvalido))?;
        conferir_pda_pelo_bump_gravado(
            info.key,
            [FEE_SHARE_SEED, socio.as_ref()],
            livro.bump,
            DomError::LivroDoSocioInvalido,
        )?;
        livro
    };
    livro.socio = *socio;
    livro.shares = livro
        .shares
        .checked_add(cotas)
        .ok_or(DomError::MathOverflow)?;
    let mut data = info.try_borrow_mut_data()?;
    require!(data.len() == espaco, DomError::LivroDoSocioInvalido);
    livro.try_serialize(&mut &mut data[..])?;
    Ok(())
}

/// Abre uma conta DESTE programa numa PDA, com o rent pago pelo `payer`.
///
/// ⚠️ **Extraído do item 7.2 sem mudar uma linha do que ele fazia**, porque agora
/// há duas contas a abrir no fechamento (a posição do afiliado e a reserva de
/// comissão dele) e o caminho abaixo é sutil: duplicá-lo é a forma conhecida de
/// as duas cópias saírem de sincronia.
///
/// ─── ⚠️ NEGAÇÃO DE SERVIÇO POR 1 LAMPORT — MEDIDA, NÃO SUPOSTA ──────────────
/// `create_account` RECUSA uma conta que já tem lamports:
///
///   "Create Account: account Address { address: Gzq… } already in use"
///   Program 1111…1111 failed: custom program error: 0x0
///
/// E o endereço de uma PDA é público e derivável por qualquer um. Então
/// **qualquer pessoa na rede manda 1 lamport para a PDA e derruba o fechamento
/// do ciclo inteiro** — de novo, e de novo, por fração de centavo por tentativa.
///
/// ⚠️ **ESTA CASA JÁ SOFREU ESTE ATAQUE, em outro lugar:** em 05/10 chegaram
/// QUATRO poeiras na gaveta, de dois endereços vanity construídos sobre a
/// Operacional do fundo — e foi por isso que a trava de igualdade do `p` virou
/// piso no Upgrade L (`:262`). O vetor é conhecido nesta casa.
///
/// Medido com `cargo test n72_poeira`: com o `create_account` puro, 1 lamport na
/// PDA derrubava a instrução INTEIRA. O caminho abaixo resiste: completa o rent
/// que falta, ALOCA o espaço e ATRIBUI o dono — os três passos que o
/// `create_account` faz junto, e que separados não se importam com o saldo que
/// já estava lá. A poeira FICA na conta: é dela agora.
pub(crate) fn abrir_conta_do_programa<'info>(
    info: &AccountInfo<'info>,
    seeds: &[&[u8]],
    espaco: usize,
    payer: &AccountInfo<'info>,
    system_program_id: Pubkey,
) -> Result<()> {
    let rent = Rent::get()?.minimum_balance(espaco);
    let lamports = info.lamports();
    // ⚠️ O `CpiContext` leva a CHAVE do programa, nao o AccountInfo — o mesmo
    // idioma do `update_whitelist.rs:117`, e o compilador recusa o outro.
    if lamports == 0 {
        anchor_lang::system_program::create_account(
            CpiContext::new_with_signer(
                system_program_id,
                anchor_lang::system_program::CreateAccount {
                    from: payer.clone(),
                    to: info.clone(),
                },
                &[seeds],
            ),
            rent,
            espaco as u64,
            &crate::ID,
        )?;
    } else {
        if lamports < rent {
            anchor_lang::system_program::transfer(
                CpiContext::new(
                    system_program_id,
                    anchor_lang::system_program::Transfer {
                        from: payer.clone(),
                        to: info.clone(),
                    },
                ),
                rent - lamports,
            )?;
        }
        anchor_lang::system_program::allocate(
            CpiContext::new_with_signer(
                system_program_id,
                anchor_lang::system_program::Allocate {
                    account_to_allocate: info.clone(),
                },
                &[seeds],
            ),
            espaco as u64,
        )?;
        anchor_lang::system_program::assign(
            CpiContext::new_with_signer(
                system_program_id,
                anchor_lang::system_program::Assign {
                    account_to_assign: info.clone(),
                },
                &[seeds],
            ),
            &crate::ID,
        )?;
    }
    Ok(())
}

/// Lê a reserva de comissão de um afiliado **sem criá-la**: devolve o acumulado
/// (0 quando a conta ainda não existe) e o `bump` da PDA.
///
/// ⚠️ **Não criar na leitura é de propósito:** a maioria dos afiliados recebe
/// comissão acima do mínimo e nunca precisa de reserva. Criar aqui cobraria rent
/// (0,00089916 SOL na mainnet, medido) de toda linha do fechamento para guardar
/// um zero.
fn reserva_do_afiliado<'info>(
    info: &'info AccountInfo<'info>,
    owner: &Pubkey,
) -> Result<(u64, u8)> {
    require!(info.is_writable, DomError::ReservaDeComissaoInvalida);
    if info.data_is_empty() {
        // Conta que ainda não existe: só a busca canônica prova o endereço, e o
        // bump dela é o que a `gravar_reserva` usa para criar.
        let (esperada, bump) =
            Pubkey::find_program_address(&[COMISSAO_SEED, owner.as_ref()], &crate::ID);
        require_keys_eq!(info.key(), esperada, DomError::ReservaDeComissaoInvalida);
        return Ok((0, bump));
    }
    // ⚠️ `Account::try_from` confere o DONO e o DISCRIMINADOR antes de ler campo:
    // conta de 49 bytes com o discriminador de outra struct devolveria números
    // plausíveis vindos do lugar errado.
    let reserva: Account<ReservaDeComissao> = Account::try_from(info)?;
    conferir_pda_pelo_bump_gravado(
        info.key,
        [COMISSAO_SEED, owner.as_ref()],
        reserva.bump,
        DomError::ReservaDeComissaoInvalida,
    )?;
    require_keys_eq!(reserva.owner, *owner, DomError::ReservaDeComissaoInvalida);
    Ok((reserva.acumulado_micro, reserva.bump))
}

/// Grava o acumulado na reserva do afiliado, abrindo a conta se ela ainda não
/// existir (rent do `payer`).
pub(crate) fn gravar_reserva<'info>(
    info: &'info AccountInfo<'info>,
    owner: &Pubkey,
    bump: u8,
    acumulado_micro: u64,
    payer: &AccountInfo<'info>,
    system_program_id: Pubkey,
) -> Result<()> {
    let espaco = 8 + ReservaDeComissao::INIT_SPACE;
    if info.data_is_empty() {
        let seeds: &[&[u8]] = &[COMISSAO_SEED, owner.as_ref(), &[bump]];
        abrir_conta_do_programa(info, seeds, espaco, payer, system_program_id)?;
    }
    // ⚠️ `try_serialize` E NÃO OFFSETS À MÃO: offset escrito a mão não tem amarra
    // com a struct, e trocar dois campos de lugar no `state.rs` passaria a
    // escrever no campo errado **em silêncio**.
    let nova = ReservaDeComissao {
        owner: *owner,
        acumulado_micro,
        bump,
    };
    let mut data = info.try_borrow_mut_data()?;
    require!(data.len() == espaco, DomError::ReservaDeComissaoInvalida);
    nova.try_serialize(&mut &mut data[..])?;
    Ok(())
}

/// Lê uma `PosicaoDoCotista` de `remaining_accounts`: PDA certa, dono certo,
/// gravável quando vai ser regravada.
///
/// ⚠️ **O INDICADO continua por aqui, e NÃO se cria a posição dele.** O ganho do
/// indicado depende do `indice_entrada` DELE; uma posição nova nasce com a
/// entrada de agora e o ganho sairia ZERO — a comissão viraria zero em silêncio.
/// Indicado sem posição é lista errada, e a instrução recusa.
pub(crate) fn carregar_posicao<'info>(
    info: &'info AccountInfo<'info>,
    owner: &Pubkey,
    gravavel: bool,
) -> Result<Account<'info, PosicaoDoCotista>> {
    require!(
        !gravavel || info.is_writable,
        DomError::PosicaoDoCotistaAusente
    );
    let posicao: Account<PosicaoDoCotista> = Account::try_from(info)?;
    conferir_pda_pelo_bump_gravado(
        info.key,
        [POSICAO_SEED, owner.as_ref()],
        posicao.bump,
        DomError::PosicaoDoCotistaAusente,
    )?;
    require_keys_eq!(posicao.owner, *owner, DomError::PosicaoDoCotistaAusente);
    Ok(posicao)
}

/// Cunha `cotas_base` para uma carteira que NÃO assina o fechamento (sócio,
/// afiliado), compensando o acerto pendente dela na própria cunhagem: crédito
/// cunha a mais, débito cunha a menos; débito maior que a cunhagem é
/// `AcertoPendente` (o dono assina `acertar` antes). Registra a entrada ao
/// índice de agora e anda os odômetros. Devolve o que cunhou.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cunhar_com_acerto<'info>(
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
    crate::indice::registrar_entrada(
        posicao,
        owner,
        bump,
        cotas_antes,
        cunhar,
        vault.indice_p,
        // ⚠️ AQUI É `indice_ciclo_anterior`, E TROCAR POR `indice_ciclo` TIRARIA
        // DINHEIRO DE QUEM TEM DIREITO.
        //
        // Este caminho roda DENTRO do fechamento, **depois do roll** — neste ponto
        // `indice_p == indice_ciclo` —, e a janela que vai ler esta posição é a
        // que está **abrindo agora**, `[anterior, ciclo]`. Com piso `indice_ciclo`
        // a média viraria exatamente `indice_ciclo` e apagaria a fatia do lote
        // VELHO na janela que abre: `c₀ · (ciclo − max(e₀, anterior))`.
        //
        // É por isso que o piso é **por chamador** e não uma troca geral: o
        // `deposit`, o `deposit_para` e o hook rodam no MEIO do ciclo e usam
        // `indice_ciclo`. Achado da revisão adversária de 13/10.
        vault.indice_ciclo_anterior,
    )?;
    posicao.indice_diluicao_visto = vault.indice_diluicao;
    posicao.indice_p_acertado = vault.indice_ciclo;
    Ok(cunhar)
}

/// ⚠️ A CORRENTE DOS RETRATOS — o que impede um ciclo votado errado de passar.
///
/// O cofre não tem contador de ciclo e não há byte livre para criar um
/// (`state.rs:80`), então o número vem **votado** pela mesa. Uma mesa que votasse
/// `ciclo = 99` criaria um retrato órfão e deixaria os ciclos 4..98 sem fechar —
/// e, pior, a Fase B do ciclo em curso passaria a ler um retrato que não é o dela.
///
/// O que prende é exigir que o retrato de `ciclo − 1`:
///
/// 1. seja **a PDA** `[RETRATO_SEED, (ciclo − 1).to_le_bytes()]` — conferido nos
///    **dois** caminhos, porque passar outra conta aqui é trivial;
/// 2. exista de verdade: dono = este programa, e o **discriminador** do
///    `RetratoDoCiclo` nos 8 primeiros bytes (dono certo com dados de outro tipo
///    passaria sem o segundo);
/// 3. esteja **fechado** (`aberto == false`) — decisão 4 do dono (16:55).
///
/// A única exceção é o primeiro ciclo com retrato, que não tem antecessor: aí a
/// conta vem vazia (`lamports == 0`), e **só aí** vazia é aceita.
fn conferir_retrato_anterior(
    conta: &UncheckedAccount,
    ciclo: u64,
    program_id: &Pubkey,
) -> Result<()> {
    let anterior = ciclo
        .checked_sub(1)
        .ok_or(error!(DomError::RetratoDoCicloInvalido))?;
    let info = conta.to_account_info();
    if ciclo == PRIMEIRO_CICLO_DO_RETRATO && info.data_is_empty() {
        // ⚠️ A derivacao e' conferida SEMPRE, inclusive no primeiro ciclo: sem isso,
        // bastaria passar qualquer conta vazia para o primeiro ciclo "valer" para
        // qualquer numero. Conta vazia nao tem bump gravado, entao aqui e' a busca
        // canonica — uma vez na vida do cofre.
        let (esperada, _) =
            Pubkey::find_program_address(&[RETRATO_SEED, &anterior.to_le_bytes()], program_id);
        require_keys_eq!(conta.key(), esperada, DomError::RetratoDoCicloInvalido);
        // -------------------------------------------------------------------
        // ⚠️ "NAO EXISTE" E' MEDIDO POR **DADOS**, NUNCA POR LAMPORTS
        // -------------------------------------------------------------------
        // A primeira versao disto media `lamports() == 0`, e a revisao adversaria
        // de 13/10 mostrou o que isso custava: a PDA do retrato anterior e um
        // endereco PUBLICO, derivavel do program id. Qualquer carteira podia
        // mandar **1 lamport** para ela e, dali em diante:
        //
        //   - o atalho do primeiro ciclo deixava de valer (lamports > 0);
        //   - a conferencia caia no dono, que e o System Program: RECUSA;
        //   - e o ciclo anterior nao pode ser fechado para criar o retrato que
        //     falta, porque `ciclo >= PRIMEIRO_CICLO_DO_RETRATO` o proibe.
        //
        // Resultado medido: NENHUM ciclo fechava mais, nos dois caminhos, e so um
        // upgrade sairia disso. Custo do ataque: 1 lamport.
        //
        // `data_is_empty()` nao tem essa porta: um lamport nao cria dados. E uma
        // conta de verdade (com dados) segue pelas tres conferencias abaixo.
        if info.data_is_empty() {
            return Ok(());
        }
    }

    require_keys_eq!(*info.owner, *program_id, DomError::RetratoDoCicloInvalido);
    let dados = info.try_borrow_data()?;
    // ⚠️ DESSERIALIZA, em vez de ler byte por deslocamento. Dono certo nao basta
    // (uma posicao e um livro tem o mesmo dono), e `try_deserialize` confere o
    // DISCRIMINADOR — e um leitor por offset e' exatamente o defeito que ja me
    // custou 11 testes verdes com fixture e leitor deslocados juntos.
    // ⚠️ Erro PRÓPRIO para o retrato de outro layout: ele é o único caso aqui que
    // **não é recuperável** (nem votando outro ciclo), e sair como "PDA inválida"
    // mandava o operador procurar no lugar errado.
    // Conta deste programa de OUTRO tipo (posição, livro) é PDA errada, não
    // retrato de outro layout: o discriminador decide qual dos dois erros.
    require!(
        dados.len() >= 8 && dados[..8] == *RetratoDoCiclo::DISCRIMINATOR,
        DomError::RetratoDoCicloInvalido
    );
    let r = RetratoDoCiclo::try_deserialize(&mut &dados[..])
        .map_err(|_| error!(DomError::RetratoDeOutroLayout))?;
    conferir_pda_pelo_bump_gravado(
        conta.key,
        [RETRATO_SEED, &anterior.to_le_bytes()],
        r.bump,
        DomError::RetratoDoCicloInvalido,
    )?;
    require_eq!(r.ciclo, anterior, DomError::RetratoDoCicloInvalido);
    require!(!r.aberto, DomError::RetratoAnteriorAberto);
    Ok(())
}

/// Lê o `reservas_em_aberto_micro` do retrato ANTERIOR — **DECISOES 13/10 21:00**.
///
/// O retrato anterior já foi conferido pela `conferir_retrato_anterior` (derivação,
/// dono, discriminador e fechado) antes de qualquer movimento de dinheiro. Aqui só
/// se lê o número, e conta vazia (o primeiro ciclo) devolve zero.
fn herdar_reservas(conta: &UncheckedAccount, ciclo: u64) -> Result<u64> {
    let info = conta.to_account_info();
    if info.data_is_empty() {
        // Só o primeiro ciclo chega aqui com conta vazia — a `conferir_retrato_anterior`
        // recusa qualquer outro.
        require!(
            ciclo == PRIMEIRO_CICLO_DO_RETRATO,
            DomError::RetratoDoCicloInvalido
        );
        return Ok(0);
    }
    let dados = info.try_borrow_data()?;
    let r = RetratoDoCiclo::try_deserialize(&mut &dados[..])
        .map_err(|_| error!(DomError::RetratoDeOutroLayout))?;
    Ok(r.reservas_em_aberto_micro)
}
