//! Bloco de estado da F2 — os achados da auditoria virando trava, com
//! contra-teste na mesma execução (T70–T79).
//!
//! - **DV11** staleness (26h) + sanity bound (15%) + **válvula da mesa**;
//! - **DV10** cadência da apuração;
//! - **DV12** atestação em estado;
//! - campos de soma corrente (`escrowed_usdc`, `deployed_usdc`) e a allowlist.

#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{
        constants::{
            DISTRIBUICAO_INTERVAL, MAX_NAV_STALENESS, MIN_DEPOSIT, MIN_RESGATE_CAPITAL_USDC,
            NAV_SCALE,
        },
        error::DomError,
    },
    solana_signer::Signer,
};

const APORTE: u64 = 20_000 * UNIT;
/// Lucro realizado de um ciclo, para os testes que precisam de uma
/// distribuicao acontecendo.
const LUCRO: u64 = 200 * UNIT;

// ---------------------------------------------------------------------------
// T70–T73 — DV11, staleness: NAV velho não vira dinheiro
// ---------------------------------------------------------------------------

/// T70 — `deposit` recusa NAV velho. É o ponto onde NAV errado vira **cota**
/// errada, e cota errada dilui quem já está dentro.
#[test]
fn t70_staleness_barra_deposit() {
    let mut env = Env::new();
    // Guarda 400 USDC no bolso: precisa sobrar para a tentativa recusada e para
    // o contra-teste.
    let a = env.carteira(APORTE + 400 * UNIT);
    env.deposit(&a, APORTE);
    env.publish_nav(1_100_000);

    env.avancar_sem_oraculo(MAX_NAV_STALENESS + 1);
    assert_dom_error(
        env.deposit_raw(&a, 200 * UNIT),
        DomError::NavStale,
        "T70 deposito com NAV velho",
    );

    // Contra-teste na mesma execução: o oráculo publica e o depósito passa.
    env.publish_nav(1_100_000);
    env.deposit(&a, 200 * UNIT);
}

/// T71 — **REVERTIDO: NAV velho NÃO barra o pedido de resgate.**
///
/// Na fila D+30 barrava, e com razão: o pedido congelava um `value` que valia por
/// 30 dias, e congelar contra preço morto gravava a distorção dentro do pedido.
///
/// O resgate de capital **não congela valor nenhum**. O `nav_na_solicitacao` é
/// teto do pior-NAV, não preço, e quem paga é o número que a mesa informa na
/// efetivação — que tem trava de frescura própria (T72). Sem valor congelado, o
/// motivo da trava evaporou, e o que sobra é a D-F2-03:
///
/// > **Pedido de resgate é direito do cotista — não fica refém do backend.**
///
/// Este teste registra a reversão de propósito. Quem ler o histórico tem que
/// achar a razão, não só a mudança.
#[test]
fn t71_staleness_nao_barra_o_pedido_de_resgate() {
    let mut env = Env::new();
    let a = env.cotista(APORTE);
    env.publish_nav(1_100_000);

    env.avancar_sem_oraculo(MAX_NAV_STALENESS + 1);
    let id = env.solicitar_resgate(&a, MIN_RESGATE_CAPITAL_USDC);
    assert_eq!(
        env.pedido(id).cotas,
        MIN_RESGATE_CAPITAL_USDC,
        "o oraculo caido nao tranca a porta de saida do cotista"
    );
    assert_eq!(
        env.pedido(id).nav_na_solicitacao,
        1_100_000,
        "grava o NAV que havia — e' teto, nao preco"
    );
}

/// T72 — **a efetivação para com NAV velho, e isso é decisão consciente.**
///
/// O `pior_nav` é limitado por `vault.nav`. NAV morto afrouxa esse limite e
/// deixa pagar acima do que o fundo vale hoje: quem sai leva demais e quem fica
/// paga a conta.
///
/// Parar tem saída pelo mesmo caminho que governa o cofre — a válvula do
/// `publish_nav`, que não tem intervalo mínimo nem gate de pausa. A saída está
/// sempre a dois votos de distância, e são os mesmos dois que aprovaram a
/// efetivação.
#[test]
fn t72_staleness_para_a_efetivacao() {
    let mut env = Env::new();
    let a = env.cotista(APORTE);
    let pagadora = env.caixa_da_autoridade(50_000 * UNIT);
    env.update_endereco_resgate(pagadora);
    env.publish_nav(1_100_000);

    let id = env.solicitar_resgate(&a, MIN_RESGATE_CAPITAL_USDC);
    env.avancar_sem_oraculo(MAX_NAV_STALENESS + 1);

    let authority = env.authority.insecure_clone();
    assert_dom_error(
        env.efetivar_resgate_raw(&authority, id, 1_000_000, a.usdc),
        DomError::NavStale,
        "T72 pagamento com NAV velho",
    );
    assert_eq!(
        env.pedido(id).estado,
        0,
        "o pedido continua aberto, intacto"
    );

    // Contra-teste: publicado o NAV, a efetivação anda.
    env.publish_nav(1_100_000);
    env.efetivar_resgate(id, 1_000_000, a.usdc);
    assert_eq!(env.pedido(id).estado, 1, "T72 contra-teste: pagou");
}

/// T73 — `deposit_especial` recusa NAV velho: a distribuição emite cota ao NAV,
/// e NAV velho emite cota errada.
#[test]
fn t73_staleness_barra_a_distribuicao() {
    let mut env = Env::new();
    let _a = env.cotista(APORTE);
    env.publish_nav(1_100_000);

    env.avancar_sem_oraculo(MAX_NAV_STALENESS + 1);
    let origem = env.caixa_da_autoridade(LUCRO);
    let authority = env.authority.insecure_clone();
    assert_dom_error(
        env.deposit_especial_raw(&authority, origem, LUCRO),
        DomError::NavStale,
        "T73 distribuicao com NAV velho",
    );

    env.publish_nav(1_100_000);
    env.deposit_especial(LUCRO);
}

// ---------------------------------------------------------------------------
// T74–T75 — DV11, o limite de variação e a válvula
// ---------------------------------------------------------------------------

/// T74 — **o oráculo não atravessa o limite de 15%.** É esta trava, e não o
/// staleness, que para o vetor do achado: oráculo comprometido publicando NAV de
/// pó de arroz para mintar cota infinita no depósito seguinte.
///
/// Limite **inclusivo**: 15% exatos passam, um micro-USDC além não.
#[test]
fn t74_oraculo_nao_atravessa_o_bound() {
    let mut env = Env::new();
    let _a = env.cotista(APORTE);

    // O vetor do DV11, literal.
    assert_dom_error(
        env.publish_nav_pelo_oraculo_raw(1),
        DomError::NavOutOfBounds,
        "T74 NAV de po de arroz",
    );
    assert_dom_error(
        env.publish_nav_pelo_oraculo_raw(NAV_SCALE * 100),
        DomError::NavOutOfBounds,
        "T74 NAV inflado",
    );
    // Um micro acima do limite.
    assert_dom_error(
        env.publish_nav_pelo_oraculo_raw(1_150_001),
        DomError::NavOutOfBounds,
        "T74 um micro alem dos 15%",
    );
    assert_eq!(env.vault().nav, NAV_SCALE, "nada disso mudou o NAV");

    // Contra-teste: 15% exatos passam pelo oráculo.
    assert_ok(
        env.publish_nav_pelo_oraculo_raw(1_150_000),
        "T74 contra-teste: 15% exatos",
    );
    assert_eq!(env.vault().nav, 1_150_000);
}

/// T75 — **a válvula da mesa atravessa o limite, e é ela que torna 15% seguro.**
///
/// Sem este caminho, um dia de queda real de 20% seria recusado, o NAV pararia
/// de ser publicado, 26h depois o staleness derrubaria depósito, pedido, fila e
/// apuração, e o cofre travaria exatamente na crise. O vetor continua barrado
/// porque quem tem a chave do oráculo **não tem o multisig**.
#[test]
fn t75_valvula_da_mesa_publica_fora_do_bound() {
    let mut env = Env::new();
    let _a = env.cotista(APORTE);

    // O mesmo valor que o oráculo não consegue publicar.
    assert_dom_error(
        env.publish_nav_pelo_oraculo_raw(800_000),
        DomError::NavOutOfBounds,
        "T75 oraculo tentando -20%",
    );

    // A mesa publica. É o dia de queda real.
    env.publish_nav_pela_mesa(800_000);
    assert_eq!(env.vault().nav, 800_000, "a mesa atravessou o limite");

    // E o cofre continua vivo: a fila anda, o depósito entra.
    let b = env.cotista(20_000 * UNIT);
    let id = env.solicitar_resgate(&b, MIN_RESGATE_CAPITAL_USDC);
    assert_eq!(
        env.pedido(id).cotas,
        MIN_RESGATE_CAPITAL_USDC,
        "T75 o cofre nao bricou na crise"
    );
}

// ---------------------------------------------------------------------------
// T76 — DV12, atestação em estado
// ---------------------------------------------------------------------------

/// T76 — o `attestation_hash` fica **no estado**, não só no evento. Evento some
/// da janela de retenção do RPC e leva junto a prova de qual laudo sustentava
/// qual NAV; backend que reindexa do zero não reconstrói isso.
#[test]
fn t76_atestacao_fica_no_estado() {
    // Gênese crua: o `Env::new` já publica NAV (T80), e aqui o que interessa é
    // o estado ANTES de qualquer laudo existir.
    let mut env = Env::sem_nav_publicado();
    assert_eq!(
        env.vault().nav_attestation,
        [0u8; 32],
        "genese nao tem laudo: o NAV e o de partida"
    );

    let oracle = env.oracle.insecure_clone();
    env.avancar_sem_oraculo(dom_vault::constants::MIN_NAV_PUBLISH_INTERVAL);
    let t = env.agora();
    assert_ok(
        env.publish_nav_raw_com_hash(&oracle, 1_100_000, [42u8; 32], t),
        "T76 publicacao com laudo",
    );
    assert_eq!(env.vault().nav_attestation, [42u8; 32], "laudo gravado");

    // Publicação nova troca o laudo — o estado guarda o do NAV corrente.
    env.avancar_sem_oraculo(dom_vault::constants::MIN_NAV_PUBLISH_INTERVAL);
    let t2 = env.agora();
    assert_ok(
        env.publish_nav_raw_com_hash(&oracle, 1_150_000, [43u8; 32], t2),
        "T76 segunda publicacao",
    );
    assert_eq!(
        env.vault().nav_attestation,
        [43u8; 32],
        "laudo do NAV corrente"
    );
}

// ---------------------------------------------------------------------------
// T77 — DV10, cadência da apuração
// ---------------------------------------------------------------------------

/// T77 — **apurar duas vezes no mesmo trimestre é recusado.**
///
/// O HWM já impedia cobrar duas vezes sobre o mesmo lucro. O que ele não impedia
/// era cobrar com frequência alta demais: apurando todo dia, cada repique vira
/// taxa e as quedas entre eles não devolvem nada.
///
/// E a trava fica **depois** do desvio de "sem lucro": chamada que não cobra
/// nada não extrai nada, e não queima o trimestre de ninguém.
#[test]
fn t77_cadencia_da_distribuicao() {
    let mut env = Env::new();
    let _a = env.cotista(APORTE);
    let authority = env.authority.insecure_clone();

    env.publish_nav(1_100_000);
    env.deposit_especial(LUCRO); // primeira: nao espera quinzena nenhuma
    let supply_1 = env.supply_dom();
    let carimbo = env.vault().ultima_distribuicao_ts;
    assert!(carimbo > 0, "carimbou a distribuicao");

    // Lucro novo na mesma quinzena: recusado.
    let origem = env.caixa_da_autoridade(LUCRO);
    assert_dom_error(
        env.deposit_especial_raw(&authority, origem, LUCRO),
        DomError::DistribuicaoMuitoCedo,
        "T77 segunda distribuicao na mesma quinzena",
    );
    assert_eq!(env.supply_dom(), supply_1, "nao mintou nada");
    assert_eq!(
        env.vault().ultima_distribuicao_ts,
        carimbo,
        "carimbo intacto"
    );

    // Contra-teste: passada a quinzena, distribui.
    env.avancar(DISTRIBUICAO_INTERVAL);
    env.deposit_especial(LUCRO);
    assert!(env.supply_dom() > supply_1, "T77 contra-teste: distribuiu");
    assert!(
        env.vault().ultima_distribuicao_ts > carimbo,
        "carimbo andou"
    );
}

/// T78 — ciclo **sem lucro realizado** não queima a quinzena.
///
/// Na régua antiga isto era um desvio interno: o `accrue_performance` aceitava a
/// chamada, não mintava nada e não carimbava. Agora é mais simples e mais
/// honesto — sem `P` não há chamada nenhuma a fazer, e o carimbo fica onde
/// estava sozinho. O que o teste prova é o mesmo: ciclo ruim não consome a
/// cadência do ciclo seguinte.
#[test]
fn t78_ciclo_sem_lucro_realizado_nao_queima_a_quinzena() {
    let mut env = Env::new();
    let _a = env.cotista(APORTE);

    // O NAV até subiu — mas nada voltou para o caixa, então não há o que
    // distribuir e ninguém chama a instrução.
    env.publish_nav(1_100_000);
    assert_eq!(
        env.vault().ultima_distribuicao_ts,
        0,
        "nao carimbou: nao houve distribuicao"
    );

    // E a primeira distribuição de verdade continua sem esperar quinzena.
    let supply_antes = env.supply_dom();
    env.deposit_especial(LUCRO);
    assert!(env.supply_dom() > supply_antes, "distribuiu sem esperar");
}

// ---------------------------------------------------------------------------
// T79 — somas correntes e a allowlist
// ---------------------------------------------------------------------------

/// T79 — `cotas_travadas_resgate` acompanha os pedidos, mantido pelas duas pontas.
///
/// Sucede o `escrowed_usdc`, que era o passivo da fila **em USDC** e saiu junto
/// com ela. O contador novo é **em cotas**, e a diferença não é cosmética: o
/// passivo antigo existia para a trava de reserva do `deploy_capital` não varrer
/// a fila; este existe para o escrow poder ser conferido contra o livro sem
/// depender de preço nenhum — a invariante do fuzz é `escrow.amount == contador`,
/// e ela vale com qualquer NAV.
#[test]
fn t79_cotas_travadas_acompanham_os_pedidos() {
    let mut env = Env::new();
    let a = env.cotista(APORTE);
    let pagadora = env.caixa_da_autoridade(200_000 * UNIT);
    env.update_endereco_resgate(pagadora);
    assert_eq!(
        env.vault().cotas_travadas_resgate,
        0,
        "cofre novo, nada travado"
    );

    let id_a = env.solicitar_resgate(&a, MIN_RESGATE_CAPITAL_USDC);
    assert_eq!(env.vault().cotas_travadas_resgate, MIN_RESGATE_CAPITAL_USDC);
    assert_eq!(
        env.saldo_escrow(),
        MIN_RESGATE_CAPITAL_USDC,
        "livro bate com o real"
    );

    // Segunda carteira **e** segundo pedido da mesma: parcial é permitido, e o
    // teto de um pedido por carteira morreu com a fila (D-F2-13 governava ela).
    let b = env.cotista(APORTE);
    let id_b = env.solicitar_resgate(&b, MIN_RESGATE_CAPITAL_USDC);
    let id_a2 = env.solicitar_resgate(&a, MIN_RESGATE_CAPITAL_USDC);
    assert_eq!(
        env.vault().cotas_travadas_resgate,
        MIN_RESGATE_CAPITAL_USDC * 3,
        "tres pedidos, dois deles da mesma carteira"
    );
    assert_eq!(env.saldo_escrow(), MIN_RESGATE_CAPITAL_USDC * 3);

    let nav = env.vault().nav;
    for id in [id_a, id_b, id_a2] {
        let dono = if id == id_b { b.usdc } else { a.usdc };
        env.efetivar_resgate(id, nav, dono);
    }
    assert_eq!(
        env.vault().cotas_travadas_resgate,
        0,
        "contador zerado junto com o escrow"
    );
    assert_eq!(env.saldo_escrow(), 0);
}

/// T79b — a allowlist de destinos **nasce vazia**, e `deployed_usdc` nasce zero.
///
/// Capital novo nasce preso: antes de a mesa registrar destino por proposta,
/// `deploy_capital` não tem para onde mandar. E são **duas** vagas, não três —
/// `SAFE-BASE` é endereço EVM e não pode ser destino de `transfer_checked`.
#[test]
fn t79b_allowlist_nasce_vazia() {
    let env = Env::new();
    let v = env.vault();
    assert_eq!(v.deployed_usdc, 0, "nada em campo na genese");
    assert_eq!(v.deploy_allowlist.len(), 2, "duas vagas: Solana, nao Base");
    for (i, destino) in v.deploy_allowlist.iter().enumerate() {
        assert_eq!(
            *destino,
            anchor_lang::prelude::Pubkey::default(),
            "vaga {i} tem que nascer livre"
        );
    }
}

// ---------------------------------------------------------------------------
// T80 — DV11, o canto de gênese (decisão da mesa, 2026-08-20)
// ---------------------------------------------------------------------------

/// T80 — **cofre sem NAV publicado não aceita aporte.**
///
/// Era o único canto do DV11 sem cobertura, e eu o havia registrado como buraco
/// conhecido no F2-02: o NAV de fundação (1,000000) não é preço de oráculo, é
/// preço de partida, e o `staleness` não tem o que medir contra ele — então um
/// cofre onde o oráculo **nunca** publicou aceitaria depósito ao preço de
/// partida para sempre.
///
/// A mesa mandou fechar em vez de documentar. O aporte agora exige que exista
/// pelo menos uma publicação: `nav_ts != 0`.
#[test]
fn t80_deposit_exige_nav_publicado() {
    let mut env = Env::sem_nav_publicado();
    let a = env.carteira(1_000 * UNIT);
    env.whitelist(&a.wallet.pubkey(), true);

    assert_eq!(env.vault().nav_ts, 0, "genese crua: ninguem publicou");
    assert_dom_error(
        env.deposit_raw(&a, 200 * UNIT),
        DomError::NavNeverPublished,
        "T80 aporte em cofre sem NAV publicado",
    );
    assert_eq!(env.supply_dom(), 0, "nao mintou cota nenhuma");

    // Contra-teste na mesma execução: publicado o primeiro NAV, o aporte entra.
    env.publish_nav(NAV_SCALE);
    assert!(env.vault().nav_ts > 0, "agora existe publicacao");
    env.deposit(&a, 200 * UNIT);
    assert_eq!(
        env.supply_dom(),
        200 * UNIT,
        "T80 contra-teste: aporte entrou"
    );
}

// ---------------------------------------------------------------------------
// T90–T92 — mínimos ajustáveis e a correção do pedido de valor zero
// ---------------------------------------------------------------------------

/// T90 — **o mínimo de resgate e o NAV mínimo são coerentes.**
///
/// O `require!(valor > 0)` da efetivação existe contra um pedido cujo valor
/// trunque a zero: cota queimada por nada.
///
/// **Ele é inalcançável enquanto o mínimo for 5.000 cotas.** Para truncar a zero
/// seria preciso `cotas < NAV_SCALE / nav`; com o NAV mínimo possível (1, ou
/// 0,000001), isso dá menos de **uma** cota, e o mínimo exige 5.000. Este teste
/// afirma a coerência entre as duas constantes — se alguém baixar o
/// `MIN_RESGATE_CAPITAL_USDC` no futuro, é aqui que a incompatibilidade aparece.
#[test]
fn t90_minimo_de_resgate_e_nav_minimo_sao_coerentes() {
    let mut env = Env::new();
    let a = env.cotista(APORTE);
    let pagadora = env.caixa_da_autoridade(200_000 * UNIT);
    env.update_endereco_resgate(pagadora);

    // NAV de pó: 0,000001 — o menor que o programa aceita (`nav > 0`).
    env.publish_nav(1);

    // No pior NAV possível, o pedido mínimo ainda vale mais que zero.
    let menor_valor = (MIN_RESGATE_CAPITAL_USDC as u128) / (NAV_SCALE as u128);
    assert!(
        menor_valor > 0,
        "MIN_RESGATE_CAPITAL_USDC ({MIN_RESGATE_CAPITAL_USDC}) ao NAV minimo trunca para \
         zero — o `require!(valor > 0)` da efetivacao passaria a barrar resgate \
         legitimo, e o cotista ficaria preso"
    );

    let id = env.solicitar_resgate(&a, MIN_RESGATE_CAPITAL_USDC);
    let antes = env.saldo_usdc(&a);
    env.efetivar_resgate(id, 1, a.usdc);
    assert_eq!(
        env.saldo_usdc(&a) - antes,
        menor_valor as u64,
        "pagou o truncado, e o truncado nao e' zero"
    );
}

/// T91 — o piso de aporte vem do **estado**, e só a autoridade o move.
#[test]
fn t91_min_deposit_e_campo_ajustavel() {
    let mut env = Env::new();
    let a = env.carteira(5_000 * UNIT);
    env.whitelist(&a.wallet.pubkey(), true);

    // Nasce no valor de gênese.
    assert_eq!(env.vault().min_deposit, MIN_DEPOSIT);
    assert_dom_error(
        env.deposit_raw(&a, MIN_DEPOSIT - 1),
        DomError::DepositBelowMinimum,
        "T91 abaixo do piso de genese",
    );

    // Intruso não mexe.
    let intruso = solana_keypair::Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), SOL).unwrap();
    assert_dom_error(
        env.update_min_deposit_raw(&intruso, 1_000_000),
        DomError::Unauthorized,
        "T91 intruso mudando o piso",
    );
    assert_eq!(env.vault().min_deposit, MIN_DEPOSIT, "piso intacto");

    // Zero não é piso: abriria aporte de poeira.
    assert_dom_error(
        env.update_min_deposit_raw(&env.authority.insecure_clone(), 0),
        DomError::InvalidMinDeposit,
        "T91 piso zero",
    );

    // A mesa baixa para 1 USDC e o aporte pequeno passa.
    env.update_min_deposit(UNIT);
    assert_eq!(env.vault().min_deposit, UNIT);
    env.deposit(&a, UNIT);
    assert!(
        env.saldo_dom(&a) > 0,
        "T91 contra-teste: aporte pequeno entrou"
    );

    // -----------------------------------------------------------------------
    // E a mesa sobe de novo, que é a estratégia de produção.
    //
    // ⚠️ A CONFERÊNCIA PASSOU A SER COM CARTEIRA NOVA — `D-F2-33`.
    //
    // Este ensaio barrava `a`, e `a` já tinha aportado `UNIT` acima. A partir do
    // momento em que o piso é DE ENTRADA, `a` é cotista e não é mais barrada —
    // então o ensaio, como estava, provava o comportamento antigo.
    //
    // Não foi enfraquecido: passou a provar as DUAS metades, que é o que a regra
    // de fato afirma. Se alguém reverter a mudança, a segunda falha; se alguém
    // apagar o piso por engano, a primeira falha.
    // -----------------------------------------------------------------------
    env.update_min_deposit(5_000 * UNIT);

    let nova = env.carteira(5_000 * UNIT);
    assert_dom_error(
        env.deposit_raw(&nova, UNIT),
        DomError::DepositBelowMinimum,
        "T91 piso alto barra quem ESTA ENTRANDO",
    );

    let antes = env.saldo_dom(&a);
    assert_ok(
        env.deposit_raw(&a, UNIT),
        "T91 piso alto NAO barra quem ja' e' cotista",
    );
    assert!(
        env.saldo_dom(&a) > antes,
        "T91 e o reaporte emitiu cota de verdade"
    );
}

/// O tamanho da conta do cofre, afirmado em número.
///
/// Existe porque a `migrate_vault_lucro` carrega offsets **literais** — 376 do
/// `hwm` que saiu, 508 dos bumps, 511 do `min_deposit` no layout antigo. Se um
/// campo novo entrar no meio do struct em vez de no fim, aqueles números viram
/// mentira e a migração desloca a cauda para o lugar errado. Este teste é o
/// alarme: campo novo no fim muda o total aqui e a linha se atualiza; campo no
/// meio muda o total **e** invalida a migração, e aí a conversa é outra.
#[test]
fn tamanho_da_conta_do_cofre() {
    use anchor_lang::Space;
    assert_eq!(
        8 + dom_vault::state::Vault::INIT_SPACE,
        731,
        "layout do cofre mudou — confira a tabela de offsets da migrar_vault_indice (Upgrade J: 603 → 731)"
    );

    let env = Env::new();
    let conta = env.svm.get_account(&vault_pda()).unwrap();
    assert_eq!(
        conta.data.len(),
        731,
        "a conta gravada tem o tamanho do struct"
    );
}

// ---------------------------------------------------------------------------
// O buraco que a guarda de tamanho NÃO fecha
// ---------------------------------------------------------------------------

/// Dois layouts do MESMO tamanho: o `Account<Vault>` **não** os distingue.
///
/// A migração `519 → 527` foi segura porque o **tamanho mudou**: o programa novo
/// pedia 527, achava 519, o Borsh ficava sem bytes — `3003`, falha fechada.
///
/// Este pacote passou a **um campo** de não ter essa sorte:
///
/// ```text
/// executado:    519 − 8 (hwm) + 16 (dois campos novos) = 527   tamanho MUDOU
/// se fosse um:  519 − 8 (hwm) +  8 (um campo novo)     = 519   tamanho IGUAL
/// ```
///
/// O discriminador do Anchor identifica **tipo**, não **versão**. Com tamanho
/// igual e discriminador igual, não sobra nada para o `Account` conferir — e este
/// teste prova que **a desserialização passa**, entregando campos trocados.
///
/// # O que barra hoje, e por que não basta
///
/// Duas travas pegam o caso, e **nenhuma das duas foi feita para isto**:
///
/// 1. **`paused: bool`** só aceita `0` ou `1`. Um layout deslocado costuma cair
///    num byte inválido — foi exatamente isso que produziu o `Invalid bool: 15`.
///    Depende do **valor** que calha de cair no byte 378. Aqui escolhemos um que
///    passa.
/// 2. **`seeds = [VAULT_SEED], bump = vault.bump`** reancora a PDA no bump lido
///    do próprio estado. Bump trocado ⇒ endereço derivado diferente ⇒ `2006
///    ConstraintSeeds`. É o que barra neste teste. Também depende do valor: se o
///    byte deslocado calhar de ser o bump verdadeiro, passa.
///
/// As duas são **acidentais e dependentes de dado**, e nenhuma protege quem só
/// **lê** a conta — o dashboard, o backend de NAV, qualquer indexador. Para
/// esses, a desserialização bem-sucedida com valores trocados é o resultado
/// final, e é silenciosa.
///
/// É o argumento para um **campo de versão de layout** carimbado no struct, que
/// o programa confira: troca duas travas acidentais por uma determinística.
#[test]
fn layout_do_mesmo_tamanho_desserializa_e_so_uma_trava_acidental_barra() {
    let mut env = Env::new();

    let bump_certo = env.vault().bump;
    let min_deposit_certo = env.vault().min_deposit;
    assert_eq!(min_deposit_certo, MIN_DEPOSIT);

    let atual = env.bytes_do_cofre();
    assert_eq!(atual.len(), 731);

    // `paused` mora no byte 378 e `cap_enforced` no 379 — aqui, o 3º e o 4º byte
    // deste u64. Com `1`, os dois viram `0x00`: booleanos válidos, e a trava (1)
    // não dispara. Com `1_000_000` o 3º byte seria `0x0F` e o Borsh recusaria —
    // a sorte que o layout de produção teve.
    const CAMPO_RESSUSCITADO: u64 = 1;

    let mut alternativo = Vec::with_capacity(731);
    alternativo.extend_from_slice(&atual[..344]);
    alternativo.extend_from_slice(&CAMPO_RESSUSCITADO.to_le_bytes());
    alternativo.extend_from_slice(&atual[344..723]);
    assert_eq!(alternativo.len(), 731, "mesmo tamanho da conta viva");

    let mut conta = env.svm.get_account(&vault_pda()).unwrap();
    conta.data = alternativo;
    env.svm.set_account(vault_pda(), conta).unwrap();

    // A DESSERIALIZAÇÃO PASSA. Nada de `3003`. É este o achado.
    let lido = env.vault();
    assert_ne!(
        lido.bump, bump_certo,
        "o bump veio de outro lugar — e é ele que faz o PDA assinar"
    );
    assert_ne!(
        lido.min_deposit, min_deposit_certo,
        "o piso de aporte veio de outro lugar"
    );

    // Quem barra é a reancoragem da PDA, não a validação do struct.
    let ts = env.agora() + 1;
    let erro = env
        .publish_nav_raw(&env.oracle.insecure_clone(), NAV_SCALE, ts)
        .expect_err("o bump trocado tem que quebrar a derivacao da PDA");
    let logs = format!("{erro:?}");
    assert!(
        logs.contains("ConstraintSeeds"),
        "esperava 2006 ConstraintSeeds — a trava acidental que sobrou; veio: {logs}"
    );
}

/// **`redeem_fee_share` RECUSA com NAV vencido.** (D4, Upgrade C — 2026-08-26)
///
/// Este teste nasceu ao contrário: em 2026-08-26, conferindo para o dossiê da
/// auditoria a afirmação de que a trava de NAV velho cobria **todos** os caminhos
/// que viram NAV em dinheiro, descobri que cobria **cinco de seis**. O
/// `redeem_fee_share` — o caminho de pagamento do **sócio** — pagava.
///
/// O teste foi escrito primeiro provando o furo, e é agora o contra-teste da
/// correção. Fica assim de propósito: um teste que só afirma o comportamento certo
/// não conta que houve um errado.
///
/// **Contraste com o `carimbar_nav`**, que lê NAV e **não** trava, e está certo
/// assim — ver o doc-comment de `handle_carimbar_nav`. Ausência de trava não é
/// defeito por si: é defeito quando o caminho **paga**.
#[test]
fn redeem_fee_share_recusa_com_nav_vencido() {
    let mut env = Env::new();
    let _a = env.cotista(APORTE);
    env.deposit_especial(LUCRO);
    env.fechar_janela_de_lucro();

    let socio = Holder {
        wallet: env.socios[0].wallet.insecure_clone(),
        dom: env.socios[0].dom,
        usdc: env.socios[0].usdc,
    };
    let cotas = env.ledger(&socio.wallet.pubkey()).shares;
    assert!(cotas > 0, "o socio tem fee_share no livro");

    // NAV vencido: passa das 26h sem o oraculo republicar.
    env.avancar_sem_oraculo(MAX_NAV_STALENESS + 1);
    let idade = env.agora() - env.vault().nav_ts;
    assert!(idade > MAX_NAV_STALENESS, "NAV vencido: {idade}s");

    let antes = env.saldo_usdc(&socio);
    assert_dom_error(
        env.redeem_fee_share_raw(&socio, cotas),
        DomError::NavStale,
        "taxa do socio ao NAV vencido",
    );
    assert_eq!(env.saldo_usdc(&socio), antes, "nada saiu do caixa");
    assert_eq!(
        env.ledger(&socio.wallet.pubkey()).shares,
        cotas,
        "o livro do socio continua intacto — o direito nao some, so espera"
    );

    // Contra-teste: republicado o NAV, o socio recebe.
    env.publish_nav(env.vault().nav);
    assert_ok(
        env.redeem_fee_share_raw(&socio, cotas),
        "com NAV fresco, a taxa e' paga normalmente",
    );
    assert!(env.saldo_usdc(&socio) > antes);
}
