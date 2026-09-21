//! Grupo F da matriz — **distribuição de lucro e `fee_share`** (T38–T45), mais
//! o T14 (isenção do cap) e o T61 (destinos duplicados).
//!
//! A régua mudou no D-F2-09: a base da taxa deixou de ser "o que passou do high
//! water mark" e passou a ser `P`, o lucro **realizado** que entra em caixa no
//! próprio `deposit_especial`. Os testes de HWM (T39/T40/T41) foram reescritos,
//! não apagados — cada um passou a provar a regra nova no mesmo lugar onde
//! provava a antiga.
//!
//! A conta continua diluindo: a taxa não sai do fundo, vira cota nova para os
//! sócios. Por isso toda asserção aqui olha **os dois lados** — o que os sócios
//! ganharam e o que os cotistas passaram a ter por cota.

#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{
        constants::{DISTRIBUICAO_INTERVAL, NUM_SOCIOS, PERF_FEE_BPS_TOTAL},
        error::DomError,
        events::LucroDistribuido,
    },
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const APORTE: u64 = 1_000 * UNIT;
/// Lucro realizado de um ciclo. 60% = 120 aos sócios, 40% = 80 aos cotistas —
/// os mesmos números da matriz antiga, agora saindo de `P` em vez do NAV.
const LUCRO: u64 = 200 * UNIT;

// ---------------------------------------------------------------------------
// T38 — a distribuição
// ---------------------------------------------------------------------------

/// `P` = 200 USDC sobre 1.000 cotas, com a taxa de gênese —
/// **`D-F2-35`: 50% no TOTAL, e não 20% por sócio.** Upgrade J: o P já está na
/// gaveta quando o oráculo publica o piso, (1.000 + 200) ÷ 1.000 = 1,20; o
/// fechamento não mexe no NAV publicado.
///
/// - aos sócios = 200 x 50% = 100 USDC; por sócio = 100 / 3 = 33,333333
/// - a sobra da divisão por três (1 lamport) vai para os cotistas
/// - aos cotistas = 100,000001 — o bolo da janela
/// - nav_fechamento = 1,20 − 100 / 1.000 = 1,100000 (pós-diluição)
#[test]
fn t38_deposit_especial_minta_fee_share_nas_tres_carteiras() {
    let mut env = Env::new();
    let cotista = env.cotista(APORTE);

    let caixa_antes = env.caixa();
    env.publish_nav_pela_mesa(1_200_000);
    let meta = env.deposit_especial(LUCRO);
    assert!(tem_evento::<LucroDistribuido>(&meta));

    let vault = env.vault();
    assert_eq!(
        vault.nav, 1_200_000,
        "o fechamento nao mexe no NAV publicado"
    );
    assert_eq!(
        vault.nav_fechamento, 1_100_000,
        "o preco pos-diluicao, congelado"
    );
    assert_eq!(
        vault.delta_lucro_por_cota, 0,
        "a regua por cota morreu no J"
    );
    assert_eq!(
        vault.lucro_sacavel_restante,
        100 * UNIT + 1,
        "o bolo e' a parcela dos cotistas, com a sobra da divisao por tres (D-F2-35)"
    );
    assert!(vault.ultima_distribuicao_ts > 0, "carimbo da distribuicao");

    // O `P` entrou de verdade no caixa — é essa a diferença para a régua antiga.
    assert_eq!(
        env.caixa(),
        caixa_antes + LUCRO,
        "o lucro realizado entrou na treasury na mesma transacao"
    );

    // 33,333333 USDC por sócio ao nav_fechamento de 1,100000 = 30,303030 cotas.
    // As cotas saem ao NAV pós-diluição, senão a emissão criaria cota de
    // graça e abriria um buraco do tamanho da parcela dos sócios.
    let esperado = 30_303_030;
    for socio in env.socios.iter() {
        assert_eq!(
            saldo(&env.svm, &socio.dom),
            esperado,
            "cada socio recebe a mesma coisa — a divisao por tres e exata entre eles"
        );
        assert_eq!(
            env.ledger(&socio.wallet.pubkey()).shares,
            esperado,
            "livro de fee_share acompanha o mint"
        );
    }

    assert_eq!(env.saldo_dom(&cotista), APORTE, "cotas do cotista intactas");
    assert_eq!(env.supply_dom(), APORTE + 3 * esperado);

    // O patrimônio fecha: (supply + cotas novas) x nav_fechamento = 1.000 + 200.
    let patrimonio = env.supply_dom() as u128 * vault.nav_fechamento as u128 / UNIT as u128;
    assert!(
        (1_200 * UNIT as u128).abs_diff(patrimonio) < UNIT as u128 / 100,
        "patrimonio preservado: {patrimonio}"
    );
}

// ---------------------------------------------------------------------------
// T39 / T40 / T41 — a régua nova, nos lugares onde o HWM era provado
// ---------------------------------------------------------------------------

/// **O teste que fecha o defeito.** NAV subindo sozinho — marcação, sem
/// dinheiro nenhum voltando ao caixa — não paga taxa a ninguém.
///
/// Na régua antiga isto mintava: o `accrue_performance` cobrava 60% de
/// `nav - hwm`, e o `redeem_fee_share` transformava essa cota em USDC de
/// verdade saindo da treasury na hora. Marcação virava caixa saindo do cofre.
/// Agora não existe instrução que cobre sobre NAV: a única porta é `P`.
#[test]
fn t39_valorizacao_de_nav_sozinha_nao_paga_taxa() {
    let mut env = Env::new();
    let _cotista = env.cotista(APORTE);

    // O NAV dobra. Nenhum dinheiro voltou para o caixa.
    env.publish_nav(1_150_000);
    env.publish_nav(1_320_000);

    assert_eq!(
        env.supply_dom(),
        APORTE,
        "nada mintado: nao ha instrucao que cobre taxa sobre NAV"
    );
    for socio in env.socios.iter() {
        assert_eq!(saldo(&env.svm, &socio.dom), 0);
        assert!(env.ledger_ausente(&socio.wallet.pubkey()));
    }
    assert_eq!(env.vault().lucro_sacavel_restante, 0, "janela fechada");
}

/// Distribuição de valor zero é recusada. `P > 0` é pré-condição, e não um
/// desvio silencioso como era o "sem lucro acima do HWM".
#[test]
fn t40_distribuicao_de_lucro_zero_e_recusada() {
    let mut env = Env::new();
    let _cotista = env.cotista(APORTE);

    let origem = env.caixa_da_autoridade(LUCRO);
    let authority = env.authority.insecure_clone();
    assert_dom_error(
        env.deposit_especial_raw(&authority, origem, 0),
        DomError::ZeroLucro,
        "T40 distribuicao de P zero",
    );
    assert_eq!(env.supply_dom(), APORTE, "nada mintado");
}

/// A base é `P` e **só** `P` — o NAV do momento não entra na conta da taxa.
///
/// Duas distribuições do mesmo `P` com NAVs bem diferentes pagam ao sócio
/// exatamente o mesmo USDC. Na régua antiga a segunda cobraria sobre toda a
/// subida acumulada acima do HWM.
#[test]
fn t41_a_base_e_o_p_do_ciclo_nao_a_subida_do_nav() {
    let mut env = Env::new();
    let _cotista = env.cotista(APORTE);

    // o piso com o P dentro: (1.000 + 200) ÷ 1.000
    env.publish_nav_pela_mesa(1_200_000);
    env.deposit_especial(LUCRO);
    let por_socio_1 = env.ledger(&env.socios[0].wallet.pubkey()).shares;
    let nav_1 = env.vault().nav_fechamento;
    let valor_1 = por_socio_1 as u128 * nav_1 as u128 / UNIT as u128;

    // NAV sobe forte por marcação (o P novo ja' dentro), e a quinzena passa.
    env.publish_nav_pela_mesa(1_500_000);
    env.avancar(DISTRIBUICAO_INTERVAL);

    let ledger_antes = env.ledger(&env.socios[0].wallet.pubkey()).shares;
    env.deposit_especial(LUCRO);
    let nav_2 = env.vault().nav_fechamento;
    let por_socio_2 = env.ledger(&env.socios[0].wallet.pubkey()).shares - ledger_antes;
    let valor_2 = por_socio_2 as u128 * nav_2 as u128 / UNIT as u128;

    // `D-F2-35`: o campo e' o TOTAL, e a divisao por tres vem depois.
    let esperado = (LUCRO as u128 * PERF_FEE_BPS_TOTAL as u128) / 10_000 / NUM_SOCIOS as u128;
    assert!(
        valor_1.abs_diff(esperado) <= 2 && valor_2.abs_diff(esperado) <= 2,
        "cada distribuicao paga a fatia do socio, em USDC: {valor_1} e {valor_2} contra {esperado}"
    );
    assert!(
        por_socio_2 < por_socio_1,
        "com NAV maior, o mesmo USDC compra MENOS cota — {por_socio_2} contra {por_socio_1}"
    );
}

/// DV10 — a cadência. Quinzenal, e a **primeira** não espera.
#[test]
fn cadencia_quinzenal_barra_a_distribuicao_apressada() {
    let mut env = Env::new();
    let _cotista = env.cotista(APORTE);

    env.deposit_especial(LUCRO); // a primeira passa: nunca distribuiu

    env.avancar(DISTRIBUICAO_INTERVAL - 1);
    let origem = env.caixa_da_autoridade(LUCRO);
    let authority = env.authority.insecure_clone();
    assert_dom_error(
        env.deposit_especial_raw(&authority, origem, LUCRO),
        DomError::DistribuicaoMuitoCedo,
        "distribuicao um segundo antes da quinzena",
    );

    // Um segundo depois, passa.
    env.avancar(1);
    env.deposit_especial(LUCRO);
}

// ---------------------------------------------------------------------------
// T14 — `fee_share` é isento do cap
// ---------------------------------------------------------------------------

/// Sócio acima de 25% do supply **continua recebendo** `fee_share`: o cap existe
/// para proteger a fila D+30 de um cotista dominante, e `fee_share` não usa a
/// fila (D3.1).
///
/// O contra-teste é o T15 na mesma execução: o mesmo sócio, no mesmo instante,
/// **não** consegue fazer depósito comum.
#[test]
fn t14_fee_share_e_isento_do_cap_mas_deposito_comum_nao() {
    let mut env = Env::new();

    // O sócio 0 vira cotista dominante: 800 de 1.000.
    let socio = env.socios[0].wallet.insecure_clone();
    let socio_holder = Holder {
        wallet: socio.insecure_clone(),
        dom: env.socios[0].dom,
        usdc: env.socios[0].usdc,
    };
    let usdc_mint = env.usdc_mint;
    let usdc_authority = env.usdc_authority.insecure_clone();
    mint_tokens(
        &mut env.svm,
        &env.payer,
        &token_classic(),
        &usdc_mint,
        &usdc_authority,
        &socio_holder.usdc,
        1_000 * UNIT,
    );
    env.deposit(&socio_holder, 800 * UNIT);
    let _outro = env.cotista(200 * UNIT);
    env.enable_cap();

    assert!(
        env.saldo_dom(&socio_holder) as u128 * 100 > env.supply_dom() as u128 * 25,
        "socio esta acima de 25%"
    );

    // T14: a distribuição minta para ele mesmo assim.
    let antes = env.saldo_dom(&socio_holder);
    env.deposit_especial(LUCRO);
    assert!(
        env.saldo_dom(&socio_holder) > antes,
        "T14: fee_share entrou apesar do cap"
    );

    // T15: e o depósito comum do mesmo sócio cai — pelo cap, e não pela janela.
    // Fecha a janela primeiro, senão o erro que chega e' `JanelaDeLucroAberta`
    // e o teste passaria pelo motivo errado.
    env.fechar_janela_de_lucro();
    assert_dom_error(
        env.deposit_raw(&socio_holder, 200 * UNIT),
        DomError::CapExceeded,
        "T15 deposito comum de socio acima do cap",
    );
}

// ---------------------------------------------------------------------------
// T42 / T43 — resgate de `fee_share` contra caixa livre
// ---------------------------------------------------------------------------

/// Com caixa livre acima da reserva, o resgate é instantâneo: queima a cota,
/// paga o USDC, baixa o livro. Sem fila, sem D+30.
#[test]
fn t42_redeem_fee_share_com_caixa_livre_e_instantaneo() {
    let mut env = Env::new();
    let _cotista = env.cotista(APORTE);
    env.deposit_especial(LUCRO);

    let socio = Holder {
        wallet: env.socios[0].wallet.insecure_clone(),
        dom: env.socios[0].dom,
        usdc: env.socios[0].usdc,
    };
    let cotas = env.ledger(&socio.wallet.pubkey()).shares;
    let nav = env.vault().nav;
    let esperado = (cotas as u128 * nav as u128 / UNIT as u128) as u64;

    let supply_antes = env.supply_dom();
    let usdc_antes = env.saldo_usdc(&socio);
    env.redeem_fee_share(&socio, cotas);

    assert_eq!(
        env.saldo_usdc(&socio) - usdc_antes,
        esperado,
        "USDC no bolso do socio"
    );
    assert_eq!(env.saldo_dom(&socio), 0, "cota queimada");
    assert_eq!(env.supply_dom(), supply_antes - cotas);
    assert_eq!(
        env.ledger(&socio.wallet.pubkey()).shares,
        0,
        "livro baixado"
    );
    // Queimar cota e pagar o proporcional não mexe no NAV por cota.
    assert_eq!(env.vault().nav, nav, "NAV inalterado");
}

/// Sem caixa livre acima da reserva, o resgate é **bloqueado**. O `fee_share`
/// continua no livro — não vira pedido, não entra na fila D+30 (D7).
///
/// A reserva é piso aqui e **não** é piso em `process_redemptions`: é a mesma
/// reserva com regra diferente para cada lado, e é assim de propósito.
#[test]
fn t43_redeem_fee_share_sem_caixa_livre_fica_pendente() {
    let mut env = Env::new();
    let _cotista = env.cotista(APORTE);
    env.deposit_especial(LUCRO);

    let socio = Holder {
        wallet: env.socios[0].wallet.insecure_clone(),
        dom: env.socios[0].dom,
        usdc: env.socios[0].usdc,
    };
    let cotas = env.ledger(&socio.wallet.pubkey()).shares;

    // Caixa reduzido a pouco mais que a reserva (10% de ~1.200 = ~120).
    env.forcar_caixa(125 * UNIT);

    assert_dom_error(
        env.redeem_fee_share_raw(&socio, cotas),
        DomError::NoFreeCash,
        "T43 resgate sem caixa livre",
    );
    assert_eq!(
        env.ledger(&socio.wallet.pubkey()).shares,
        cotas,
        "T43: fee_share continua no livro, pendente"
    );
    assert_eq!(env.saldo_dom(&socio), cotas, "cota nao foi queimada");

    // Contra-teste: com caixa de volta, o mesmo resgate passa.
    env.forcar_caixa(1_200 * UNIT);
    env.redeem_fee_share(&socio, cotas);
    assert_eq!(env.ledger(&socio.wallet.pubkey()).shares, 0);
}

// ---------------------------------------------------------------------------
// T44 / T45 — cota comum não é `fee_share`
// ---------------------------------------------------------------------------

/// Cotista comum não tem livro de `fee_share`: a conta nem existe, e o resgate
/// instantâneo não tem por onde começar.
#[test]
fn t44_cota_comum_nao_resgata_como_fee_share() {
    let mut env = Env::new();
    let cotista = env.cotista(APORTE);

    let resultado = env.redeem_fee_share_raw(&cotista, 100 * UNIT);
    assert!(
        resultado.is_err(),
        "T44: cotista comum nao deveria resgatar como fee_share"
    );
}

/// E a cota **comum de sócio** também não. O sócio tem livro, mas o livro conta
/// só o que veio de taxa: capital próprio de sócio segue a fila D+30 como o de
/// qualquer um.
#[test]
fn t45_cota_comum_de_socio_segue_a_fila() {
    let mut env = Env::new();
    let _cotista = env.cotista(APORTE);
    env.deposit_especial(LUCRO);
    env.fechar_janela_de_lucro();

    let socio = Holder {
        wallet: env.socios[0].wallet.insecure_clone(),
        dom: env.socios[0].dom,
        usdc: env.socios[0].usdc,
    };
    let fee = env.ledger(&socio.wallet.pubkey()).shares;

    // O sócio aporta capital próprio: vira cota comum, no mesmo token account.
    let usdc_mint = env.usdc_mint;
    let usdc_authority = env.usdc_authority.insecure_clone();
    mint_tokens(
        &mut env.svm,
        &env.payer,
        &token_classic(),
        &usdc_mint,
        &usdc_authority,
        &socio.usdc,
        500 * UNIT,
    );
    env.deposit(&socio, 500 * UNIT);
    assert!(
        env.saldo_dom(&socio) > fee,
        "socio tem cota comum alem do fee_share"
    );

    // O saldo do token account é um número só — quem separa é o livro (D3.1).
    assert_dom_error(
        env.redeem_fee_share_raw(&socio, env.saldo_dom(&socio)),
        DomError::InsufficientFeeShare,
        "T45 socio tentando resgatar cota comum como fee_share",
    );

    // Contra-teste: até o saldo do livro, passa.
    env.redeem_fee_share(&socio, fee);
    assert_eq!(env.ledger(&socio.wallet.pubkey()).shares, 0);
}

// ---------------------------------------------------------------------------
// T61 / T54 — destinos duplicados e autoridade
// ---------------------------------------------------------------------------

/// Terceira camada da D10: mesmo que `initialize` e `update_socios` fossem
/// furados por um bug futuro, o runtime do Anchor 1.x recusa a mesma conta
/// mutável duas vezes na mesma instrução (A3).
#[test]
fn t61_distribuicao_com_destinos_duplicados_falha() {
    let mut env = Env::new();
    let _cotista = env.cotista(APORTE);

    let socios: Vec<_> = env.socios.iter().map(|s| s.wallet.pubkey()).collect();
    let duplicado = env.socios[0].dom;
    let contas = [duplicado, duplicado, env.socios[2].dom];

    let origem = env.caixa_da_autoridade(LUCRO);
    let authority = env.authority.insecure_clone();
    assert!(
        env.deposit_especial_com_destinos(&authority, origem, LUCRO, &contas, &socios)
            .is_err(),
        "T61: destinos duplicados deveriam falhar"
    );
    assert_eq!(env.supply_dom(), APORTE, "nada mintado");
}

/// Distribuir não é para qualquer um (T54).
#[test]
fn deposit_especial_por_nao_autoridade_rejeita() {
    let mut env = Env::new();
    let _cotista = env.cotista(APORTE);

    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), SOL).unwrap();
    let origem = env.caixa_da_autoridade(LUCRO);
    assert!(
        env.deposit_especial_raw(&intruso, origem, LUCRO).is_err(),
        "distribuicao por intruso"
    );
    assert_eq!(env.supply_dom(), APORTE, "nada mintado");
}
