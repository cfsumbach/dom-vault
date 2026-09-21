//! **A janela de saque de lucro** (D-F2-09, refeita pelo Upgrade J — D-F2-43).
//!
//! O fechamento não sobe o NAV: o `P` já estava no preço da cota desde que
//! caiu na gaveta. O que o fechamento congela é o índice do ciclo e o
//! `nav_fechamento`; o direito de cada cotista é o ganho DELE pelo índice,
//! menos a taxa da mesa — quem entrou no meio do ciclo leva só o `P` que caiu
//! depois dele. Nesta fixture há um cotista só, desde o começo, então o número
//! coincide com o da régua antiga (500 = metade de 1.000): o que muda é de
//! onde ele vem (`tests/indice_j.rs` prova a diferença com 18 carteiras).
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

// Upgrade D: o piso do saque de lucro passou a ser 200 USDC — o par (5.000 /
// 1.000) existe para o bolo (500) passar do piso.
const APORTE: u64 = 5_000 * UNIT;
/// `P` do ciclo. 50% = 500 aos sócios (166,666666 cada) e 500 ao bolo dos
/// cotistas (a sobra de 2 micro é deles, `D-F2-35`).
const LUCRO: u64 = 1_000 * UNIT;

/// Cofre com um cotista de 5.000 cotas e um `P` de 1.000 fechado.
///
/// Antes do fechamento o oráculo publica o piso com o `P` dentro:
/// (5.000 + 1.000) ÷ 5.000 = 1,200000. O fechamento cunha a mesa (500 USDC em
/// cotas) e grava `nav_fechamento` = 1,2 − 500 ÷ 5.000 = 1,100000 — o preço
/// pós-diluição, que é o que a janela usa. O NAV publicado NÃO muda.
fn cofre_com_janela_aberta() -> (Env, Holder) {
    let mut env = Env::new();
    let cotista = env.cotista(APORTE);
    env.publish_nav_pela_mesa(1_200_000);
    env.deposit_especial(LUCRO);
    assert_eq!(
        env.vault().nav,
        1_200_000,
        "§4: o fechamento nao mexe no NAV publicado"
    );
    assert_eq!(env.vault().nav_fechamento, 1_100_000);
    assert_eq!(
        env.vault().delta_lucro_por_cota,
        0,
        "a regua por cota morreu no J"
    );
    assert_eq!(env.vault().lucro_sacavel_restante, 500 * UNIT + 2);
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
///   ganho pelo indice: 5.000 cotas × 0,2 = 1.000; metade e' dele = 500 USDC
///   queima 500 / 1,10 (nav_fechamento) = 454,545454 cotas
///   sobram 4.545,454546 cotas x 1,10 = 5.000,000000 USDC — o aporte original
#[test]
fn saque_devolve_o_cotista_a_posicao_pre_distribuicao() {
    let (mut env, cotista) = cofre_com_janela_aberta();

    let usdc_antes = env.saldo_usdc(&cotista);
    let nav = env.vault().nav_fechamento;

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
    assert_eq!(
        env.vault().nav,
        1_200_000,
        "queimar nao mexe no NAV publicado"
    );

    let posicao = env.saldo_dom(&cotista) as u128 * nav as u128 / UNIT as u128;
    assert_eq!(
        posicao, APORTE as u128,
        "de volta a posicao pre-distribuicao, exata"
    );
    assert_eq!(
        env.vault().lucro_sacavel_restante,
        2,
        "bolo esvaziado — sobra a sobra de arredondamento"
    );
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
    // entao o piso nao pode ser o que o derruba. Piso publicado com o P dentro:
    // (1.000 + 2.000) ÷ 1.000 = 3,0.
    env.publish_nav_pela_mesa(3_000_000);
    env.deposit_especial(2_000 * UNIT);

    let bolo = env.vault().lucro_sacavel_restante;
    let ua = env.saldo_usdc(&a);
    let ub = env.saldo_usdc(&b);

    env.sacar_lucro(&a);
    env.sacar_lucro(&b);

    let sacado = (env.saldo_usdc(&a) - ua) + (env.saldo_usdc(&b) - ub);
    // a: 600 × 2,0 = 1.200 de ganho → 600; b: 400 × 2,0 → 400. Soma 1.000 = o bolo (com a sobra de 1 micro).
    assert_eq!(
        sacado + 1,
        bolo,
        "a soma dos saques fecha no bolo, na unidade"
    );
    assert_eq!(env.vault().lucro_sacavel_restante, 1);
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

    // No J o bolo guarda a sobra de arredondamento (2 micro), entao a janela
    // segue "aberta" e quem recusa e' a marca — nao o teto.
    let cotas_antes = env.saldo_dom(&cotista);
    assert_dom_error(
        env.sacar_lucro_raw(&cotista),
        DomError::LucroJaSacado,
        "segundo saque na mesma janela",
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
    env.publish_nav_pela_mesa(2_000_000);
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
    // o piso com o P novo dentro: (5.500 + 1.000) ÷ 5.000 = 1,30
    env.publish_nav_pela_mesa(1_300_000);
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

    // Contra-teste: fechada a janela, a mesma transferência passa — depois do
    // acerto da origem (J7: toda saida acerta antes; aqui o liquido e' zero,
    // cotista sozinho, mas o hook so' anda o odometro de quem esta' em dia).
    env.fechar_janela_de_lucro();
    env.acertar(&cotista);
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
    env.publish_nav_pela_mesa(1_100_000);
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
    // ninguem sacou: o caixa tem 6.000 e o P novo 1.000 sobre 5.454,545454 cotas
    env.publish_nav_pela_mesa(1_283_333);
    env.deposit_especial(LUCRO);

    // O bolo e' NOVO — nao a soma com o da janela anterior, que seria 1.000.
    // No J o bolo e' a parcela dos cotistas inteira (500 + a sobra), e o que
    // cada um pode sacar sai do indice — os socios, que agora tem cota, nao
    // entram na janela (`SocioForaDaJanela`), entao a parte do P deles fica.
    assert_eq!(
        env.vault().lucro_sacavel_restante,
        500 * UNIT + 2,
        "bolo NOVO, nao o acumulado"
    );

    // E o cotista saca a distribuição nova normalmente — perdeu a velha por não
    // ter aparecido, mas o valor dela ficou na cota dele.
    let usdc_antes = env.saldo_usdc(&cotista);
    env.sacar_lucro(&cotista);
    assert!(env.saldo_usdc(&cotista) > usdc_antes);
}
