//! Grupo G da matriz — **pause** (T46–T49).
//!
//! Pausa é o análogo on-chain da desmontagem (R1 do runbook): bloqueia entrada e
//! pedido novo, mas **não** bloqueia o pagamento da fila que já existe. Quem
//! pediu resgate antes da crise não pode ser punido por ela.

#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{error::DomError, events::PauseToggled},
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const APORTE: u64 = 20_000 * UNIT;

// ---------------------------------------------------------------------------
// T46 / T47 — o que a pausa bloqueia
// ---------------------------------------------------------------------------

/// `deposit` bloqueado enquanto pausado; volta a passar depois do `unpause`.
#[test]
fn t46_pause_bloqueia_deposit() {
    let mut env = Env::new();
    let cotista = env.carteira(APORTE);

    let meta = env.pause();
    assert!(tem_evento::<PauseToggled>(&meta));
    assert!(env.vault().paused);

    assert_dom_error(
        env.deposit_raw(&cotista, 500 * UNIT),
        DomError::Paused,
        "T46 deposito com cofre pausado",
    );
    assert_eq!(env.saldo_dom(&cotista), 0);

    env.unpause();
    env.deposit(&cotista, 500 * UNIT);
    assert_eq!(env.saldo_dom(&cotista), 500 * UNIT, "T46 contra-teste");
}

/// Pedido de resgate **novo** bloqueado enquanto pausado.
#[test]
fn t47_pause_bloqueia_pedido_novo() {
    let mut env = Env::new();
    let cotista = env.cotista(APORTE);

    env.pause();
    assert_dom_error(
        env.solicitar_resgate_raw(&cotista, 5_000 * UNIT),
        DomError::Paused,
        "T47 pedido novo com cofre pausado",
    );
    assert_eq!(env.vault().proximo_pedido_id, 0, "nenhum pedido abriu");

    env.unpause();
    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);
    assert_eq!(env.pedido(id).cotas, 5_000 * UNIT, "T47 contra-teste");
}

// ---------------------------------------------------------------------------
// T48 — o que a pausa NÃO bloqueia
// ---------------------------------------------------------------------------

/// **Pedido já aberto continua sendo pago com o cofre pausado.** É a diferença
/// que dá sentido à pausa: ela para o fluxo novo, não a obrigação já assumida.
///
/// O mecanismo mudou — era a fila D+30, agora é o resgate de capital —, mas a
/// decisão é a mesma e por isso o teste continua existindo: `efetivar` não tem
/// trava de `paused`, de propósito. Pausar para não honrar resgate seria usar a
/// pausa contra o cotista, e não é para isso que ela existe.
#[test]
fn t48_pause_nao_bloqueia_a_efetivacao_de_resgate_ja_aberto() {
    let mut env = Env::new();
    let cotista = env.cotista(APORTE);
    let pagadora = env.caixa_da_autoridade(20_000 * UNIT);
    env.update_endereco_resgate(pagadora);

    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);
    env.pause();

    let antes = env.saldo_usdc(&cotista);
    let nav = env.vault().nav;
    env.efetivar_resgate(id, nav, cotista.usdc);

    assert_eq!(
        env.saldo_usdc(&cotista) - antes,
        5_000 * UNIT * nav / dom_vault::constants::NAV_SCALE,
        "T48: obrigacao ja assumida e' honrada mesmo pausado"
    );
    assert_eq!(env.pedido(id).estado, 1, "pago");
    assert!(env.vault().paused, "e o cofre continua pausado");
}

/// Distribuição, saque de lucro e resgate de `fee_share` também param na pausa.
/// Não está na matriz, mas segue o mesmo racional do R1: em crise não se
/// cristaliza taxa nem se paga sócio antes de conciliar o NAV.
#[test]
fn pause_bloqueia_distribuicao_saque_de_lucro_e_fee_share() {
    let mut env = Env::new();
    let cotista = env.cotista(APORTE);
    env.publish_nav(1_200_000);
    env.deposit_especial(200 * UNIT);

    let socio = Holder {
        wallet: env.socios[0].wallet.insecure_clone(),
        dom: env.socios[0].dom,
        usdc: env.socios[0].usdc,
    };
    let cotas = env.ledger(&socio.wallet.pubkey()).shares;

    env.pause();

    let origem = env.caixa_da_autoridade(200 * UNIT);
    let authority = env.authority.insecure_clone();
    assert_dom_error(
        env.deposit_especial_raw(&authority, origem, 200 * UNIT),
        DomError::Paused,
        "distribuicao com cofre pausado",
    );
    assert_dom_error(
        env.sacar_lucro_raw(&cotista),
        DomError::Paused,
        "saque de lucro com cofre pausado",
    );
    assert_dom_error(
        env.redeem_fee_share_raw(&socio, cotas),
        DomError::Paused,
        "resgate de fee_share com cofre pausado",
    );
}

// ---------------------------------------------------------------------------
// T49 — autoridade
// ---------------------------------------------------------------------------

/// `pause`/`unpause` só pela autoridade. E cada um é idempotente ao contrário:
/// pausar o que já está pausado falha, o que evita evento duplicado no meio de
/// uma crise.
#[test]
fn t49_pause_por_nao_autoridade_rejeita() {
    let mut env = Env::new();

    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), SOL).unwrap();

    assert_dom_error(
        env.pause_raw(&intruso),
        DomError::Unauthorized,
        "T49 pause por intruso",
    );
    assert!(!env.vault().paused);

    env.pause();
    assert_dom_error(
        env.unpause_raw(&intruso),
        DomError::Unauthorized,
        "T49 unpause por intruso",
    );
    assert!(env.vault().paused, "continua pausado");

    // Repetir o estado atual falha nos dois sentidos.
    let authority = env.authority.insecure_clone();
    assert_dom_error(
        env.pause_raw(&authority),
        DomError::Paused,
        "pausar o que ja esta pausado",
    );
    env.unpause();
    assert_dom_error(
        env.unpause_raw(&authority),
        DomError::NotPaused,
        "despausar o que nao esta pausado",
    );
}
