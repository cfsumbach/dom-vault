//! **A janela de saque de lucro** (D-F2-09).
//!
//! A distribuição sobe o NAV pro-rata, então o lucro de cada cotista é
//! exatamente `cotas × delta`. É por isso que o saque não precisa saber quanto
//! nem quando cada um aportou: um número global resolve a conta para a carteira
//! inteira, e a soma sobre quem estava dentro dá a parcela dos cotistas.
//!
//! O que não é de graça são as três travas. Cada uma tem teste próprio aqui,
//! com o contra-teste ao lado — porque as três existem contra ataques que
//! **funcionam** se elas não existirem, e um teste que só prova o caminho feliz
//! não prova trava nenhuma.

#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{constants::DISTRIBUICAO_INTERVAL, error::DomError},
    solana_signer::Signer,
};

// Upgrade D: o piso do saque de lucro passou a ser 200 USDC. Com o par antigo
// (1.000 / 200) o bolo do ciclo era 80 USDC e NENHUM saque passava — a fixture
// inteira caia na trava nova.
//
// Escalados os DOIS por 5, e nao so' o `P`: a razao `parcela_cotistas / cotas`
// fica identica, entao NAV e delta nao mudam com a escala. So' os valores em
// USDC sobem junto, que e' o que a trava mede.
//
// `D-F2-35` mudou a REPARTICAO, e ai' sim NAV e delta mudaram: de 1,080000 e
// 0,080000 para 1,100000 e 0,100000, porque os cotistas passaram de 40% para
// 50% de P.
const APORTE: u64 = 5_000 * UNIT;
/// `P` do ciclo. 50% = 500 aos sócios (166,666666 cada, e 2 lamports de sobra),
/// e 500,000002 ao bolo dos cotistas — a sobra é deles (`D-F2-35`).
const LUCRO: u64 = 1_000 * UNIT;

/// Cofre com um cotista de 5.000 cotas e uma distribuição de 1.000 já feita.
/// NAV 1,100000, delta 0,100000, bolo de 500 USDC.
///
/// O bolo é `supply × delta`, e não a parcela dos cotistas: os 2 lamports de
/// sobra subiram o patrimônio e não têm dono que os saque. Ver `D-F2-35`.
fn cofre_com_janela_aberta() -> (Env, Holder) {
    let mut env = Env::new();
    let cotista = env.cotista(APORTE);
    env.deposit_especial(LUCRO);
    assert_eq!(env.vault().nav, 1_100_000);
    assert_eq!(env.vault().delta_lucro_por_cota, 100_000);
    assert_eq!(env.vault().lucro_sacavel_restante, 500 * UNIT);
    (env, cotista)
}

// ---------------------------------------------------------------------------
// O direito, e a identidade que ele preserva
// ---------------------------------------------------------------------------

/// **A conta que sustenta o desenho inteiro.**
///
/// O cotista saca `cotas × delta` em USDC e volta à posição em valor que tinha
/// **antes** da distribuição. Não é aproximação: é a álgebra de queimar
/// `valor / nav` cotas.
///
///   5.000 cotas x 0,10 = 500 USDC sacados
///   queima 500 / 1,10 = 454,545454 cotas
///   sobram 4.545,454546 cotas x 1,10 = 5.000,000000 USDC — o aporte original
#[test]
fn saque_devolve_o_cotista_a_posicao_pre_distribuicao() {
    let (mut env, cotista) = cofre_com_janela_aberta();

    let usdc_antes = env.saldo_usdc(&cotista);
    let nav = env.vault().nav;

    env.sacar_lucro(&cotista);

    assert_eq!(
        env.saldo_usdc(&cotista) - usdc_antes,
        500 * UNIT,
        "sacou 50% de P — o cotista detem todas as cotas pre-distribuicao"
    );
    assert_eq!(
        env.saldo_dom(&cotista),
        APORTE - 454_545_454,
        "cota queimada"
    );
    assert_eq!(env.vault().nav, nav, "queimar ao NAV nao mexe no NAV");

    let posicao = env.saldo_dom(&cotista) as u128 * nav as u128 / UNIT as u128;
    assert_eq!(
        posicao, APORTE as u128,
        "de volta a posicao pre-distribuicao, exata"
    );
    assert_eq!(env.vault().lucro_sacavel_restante, 0, "bolo esvaziado");
}

/// A soma do que todos podem sacar **não passa** do bolo. É a segunda cinta:
/// mesmo que o direito individual estivesse errado, o teto global segura.
#[test]
fn a_soma_dos_saques_nao_passa_do_bolo() {
    let mut env = Env::new();
    let a = env.cotista(600 * UNIT);
    let b = env.cotista(400 * UNIT);
    // `P` proprio, maior que o da fixture: com um bolo pequeno o `b` bateria no
    // piso do Upgrade D. O que este teste afirma e' o TETO global, nao o piso —
    // entao o piso nao pode ser o que o derruba.
    env.deposit_especial(2_000 * UNIT);

    let bolo = env.vault().lucro_sacavel_restante;
    let ua = env.saldo_usdc(&a);
    let ub = env.saldo_usdc(&b);

    env.sacar_lucro(&a);
    env.sacar_lucro(&b);

    let sacado = (env.saldo_usdc(&a) - ua) + (env.saldo_usdc(&b) - ub);
    assert_eq!(sacado, bolo, "a soma dos saques fecha no bolo, na unidade");
    assert_eq!(env.vault().lucro_sacavel_restante, 0);
}

// ---------------------------------------------------------------------------
// TRAVA 1 — a marca por carteira
// ---------------------------------------------------------------------------

/// **Sem a marca, esta carteira drenaria o bolo sozinha.** Pagar não zera
/// `cotas`, então `cotas × delta` continua positivo depois do saque: 80,00 na
/// primeira chamada, depois 74,07, depois 68,58... A marca é o que transforma
/// isso num erro nomeado.
#[test]
fn segundo_saque_na_mesma_janela_e_recusado() {
    let (mut env, cotista) = cofre_com_janela_aberta();

    env.sacar_lucro(&cotista);
    let carimbo = env.vault().ultima_distribuicao_ts;
    assert_eq!(
        env.marca_de_lucro(&cotista.wallet.pubkey())
            .expect("marca criada no primeiro saque")
            .ultima_distribuicao_sacada,
        carimbo,
        "a marca guarda o carimbo da distribuicao"
    );

    let cotas_antes = env.saldo_dom(&cotista);
    assert_dom_error(
        env.sacar_lucro_raw(&cotista),
        DomError::JanelaDeLucroFechada,
        "segundo saque com o bolo ja' vazio",
    );
    assert_eq!(env.saldo_dom(&cotista), cotas_antes, "nada mais queimado");
}

/// O mesmo, com o bolo **ainda cheio**: aqui quem recusa é a marca, não o teto.
/// Os dois erros são distintos de propósito — se o teste aceitasse qualquer
/// falha, a trava 1 poderia sumir sem ninguém notar.
#[test]
fn marca_recusa_mesmo_com_bolo_sobrando() {
    let mut env = Env::new();
    let a = env.cotista(600 * UNIT);
    let _b = env.cotista(400 * UNIT);
    env.deposit_especial(LUCRO);

    env.sacar_lucro(&a);
    assert!(
        env.vault().lucro_sacavel_restante > 0,
        "o bolo do B ainda esta la'"
    );
    assert_dom_error(
        env.sacar_lucro_raw(&a),
        DomError::LucroJaSacado,
        "A tentando sacar duas vezes",
    );
}

/// E a marca **libera** na distribuição seguinte: ela guarda o carimbo, não um
/// booleano. Carimbo novo, direito novo.
#[test]
fn a_marca_libera_na_distribuicao_seguinte() {
    let (mut env, cotista) = cofre_com_janela_aberta();
    env.sacar_lucro(&cotista);

    env.avancar(DISTRIBUICAO_INTERVAL);
    env.deposit_especial(LUCRO);

    let usdc_antes = env.saldo_usdc(&cotista);
    env.sacar_lucro(&cotista);
    assert!(
        env.saldo_usdc(&cotista) > usdc_antes,
        "sacou o lucro da distribuicao nova"
    );
}

// ---------------------------------------------------------------------------
// TRAVA 2 — entrada fechada na janela
// ---------------------------------------------------------------------------

/// Aporte na janela levaria lucro que o aportador não gerou: ele compraria cota
/// ao NAV já subido **e** sacaria `cotas × delta` por cima.
#[test]
fn deposit_e_recusado_com_a_janela_aberta() {
    let (mut env, _cotista) = cofre_com_janela_aberta();

    let novato = env.carteira(500 * UNIT);
    assert_dom_error(
        env.deposit_raw(&novato, 500 * UNIT),
        DomError::JanelaDeLucroAberta,
        "aporte na janela",
    );

    // Contra-teste: fechada a janela, o mesmo aporte passa.
    env.fechar_janela_de_lucro();
    env.deposit(&novato, 500 * UNIT);
    assert!(env.saldo_dom(&novato) > 0);
}

/// **A trava do hook.** Sem ela a marca por carteira não vale nada: quem já
/// sacou passa as cotas para uma carteira virgem e saca de novo. A marca é por
/// dono; a cota, não.
#[test]
fn transferencia_de_cota_e_recusada_com_a_janela_aberta() {
    let (mut env, cotista) = cofre_com_janela_aberta();
    let vizinho = env.carteira(0);

    let destino_dom = vizinho.dom;
    let destino_dono = vizinho.wallet.pubkey();
    assert!(
        env.transfer(&cotista, &destino_dom, &destino_dono, 100 * UNIT)
            .is_err(),
        "transferencia na janela deveria falhar"
    );
    assert_eq!(saldo(&env.svm, &destino_dom), 0, "nada chegou");

    // Contra-teste: fechada a janela, a mesma transferência passa.
    env.fechar_janela_de_lucro();
    env.transfer(&cotista, &destino_dom, &destino_dono, 100 * UNIT)
        .expect("fora da janela a transferencia e' normal");
    assert_eq!(saldo(&env.svm, &destino_dom), 100 * UNIT);
}

/// **Mas o pedido de resgate continua de pé.** O destino é o escrow do programa,
/// que é isento: cota entrando ali sai do saldo de quem pediu e reduz o direito
/// dele — nunca cria direito para ninguém.
///
/// É a diferença entre fechar a janela e sequestrar o direito do cotista
/// (D-F2-03).
#[test]
fn resgate_de_capital_atravessa_a_janela() {
    // Cotista grande o bastante para o mínimo de resgate (5.000 cotas), e a
    // distribuição depois — `deposit` recusa com a janela aberta, então o aporte
    // tem de vir antes.
    let mut env = Env::new();
    let cotista = env.cotista(10_000 * UNIT);
    env.deposit_especial(LUCRO);
    assert!(env.vault().lucro_sacavel_restante > 0, "janela aberta");
    let direito_antes = env.saldo_dom(&cotista);

    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);
    assert_eq!(env.pedido(id).cotas, 5_000 * UNIT, "o pedido abriu");
    assert_eq!(env.saldo_escrow(), 5_000 * UNIT);

    // E o preço que quem pediu paga por atravessar a janela: cota em escrow não
    // conta no `sacar_lucro`, que lê o saldo da carteira. Ele abriu mão do saque
    // sobre a fração travada — escolha dele, e honesta.
    assert_eq!(
        env.saldo_dom(&cotista),
        direito_antes - 5_000 * UNIT,
        "a fracao travada saiu do saldo que da direito ao saque"
    );
}

// ---------------------------------------------------------------------------
// TRAVA 3 — sócio tem porta própria
// ---------------------------------------------------------------------------

/// Sócio saca pelo `redeem_fee_share`, não pela janela (D-F2-09). As cotas que
/// ele acabou de receber existem no instante do saque e reivindicariam parte de
/// um bolo que não é dele.
#[test]
fn socio_e_recusado_na_janela_e_saca_pela_porta_dele() {
    let (mut env, _cotista) = cofre_com_janela_aberta();

    let socio = Holder {
        wallet: env.socios[0].wallet.insecure_clone(),
        dom: env.socios[0].dom,
        usdc: env.socios[0].usdc,
    };
    assert_dom_error(
        env.sacar_lucro_raw(&socio),
        DomError::SocioForaDaJanela,
        "socio na janela dos cotistas",
    );

    // A porta dele está aberta no mesmo instante.
    let cotas = env.ledger(&socio.wallet.pubkey()).shares;
    let usdc_antes = env.saldo_usdc(&socio);
    env.redeem_fee_share(&socio, cotas);
    assert!(env.saldo_usdc(&socio) > usdc_antes, "sacou pela porta dele");
}

// ---------------------------------------------------------------------------
// A janela: quem abre, quem fecha
// ---------------------------------------------------------------------------

/// Fora da janela não há o que sacar — e o erro diz isso, em vez de pagar zero.
#[test]
fn saque_fora_da_janela_e_recusado() {
    let mut env = Env::new();
    let cotista = env.cotista(APORTE);

    assert_dom_error(
        env.sacar_lucro_raw(&cotista),
        DomError::JanelaDeLucroFechada,
        "saque sem distribuicao nenhuma atras",
    );

    // Sobe o NAV por marcação: continua não havendo janela. Marcação não é
    // lucro realizado, e é essa a régua toda do D-F2-09.
    env.publish_nav(1_150_000);
    assert_dom_error(
        env.sacar_lucro_raw(&cotista),
        DomError::JanelaDeLucroFechada,
        "saque com NAV alto mas sem distribuicao",
    );
}

/// **O re-deploy fecha a janela** — é o marco do ciclo seguinte, não um
/// cronômetro (D-F2-09). O que sobrou não some: continua no preço da cota de
/// quem não sacou.
#[test]
fn deploy_capital_fecha_a_janela() {
    let (mut env, cotista) = cofre_com_janela_aberta();

    let ops = env.carteira(0);
    env.update_deploy_allowlist(0, ops.wallet.pubkey());

    let nav_antes = env.vault().nav;
    env.deploy_capital(ops.usdc, 100 * UNIT);

    assert_eq!(env.vault().lucro_sacavel_restante, 0, "janela fechada");
    assert_eq!(
        env.vault().nav,
        nav_antes,
        "o nao sacado continua no preco da cota"
    );
    assert_dom_error(
        env.sacar_lucro_raw(&cotista),
        DomError::JanelaDeLucroFechada,
        "saque depois do re-deploy",
    );
}

/// Ciclo sem re-deploy nenhum não pode travar a distribuição seguinte: a
/// própria `deposit_especial` fecha a janela velha, e só depois de a quinzena
/// ter passado — quem tinha direito teve a quinzena inteira para sacar.
#[test]
fn distribuicao_seguinte_fecha_a_janela_esquecida() {
    let (mut env, cotista) = cofre_com_janela_aberta();

    env.avancar(DISTRIBUICAO_INTERVAL);
    env.deposit_especial(LUCRO);

    // 499,996363 e nao 500: na segunda distribuicao o `supply` ja' cresceu com
    // as cotas dos socios da primeira, entao `delta = parcela / supply` trunca
    // um pouco mais. O que este ensaio afirma e' que o bolo e' NOVO — nao a soma
    // com o da janela anterior, que seria 1.000.
    assert_eq!(
        env.vault().lucro_sacavel_restante,
        499_996_363,
        "bolo NOVO, nao o acumulado"
    );

    // E o cotista saca a distribuição nova normalmente — perdeu a velha por não
    // ter aparecido, mas o valor dela ficou na cota dele.
    let usdc_antes = env.saldo_usdc(&cotista);
    env.sacar_lucro(&cotista);
    assert!(env.saldo_usdc(&cotista) > usdc_antes);
}
