//! Grupo A da matriz — **depósito e cotização** (T01–T05), mais a metade de
//! `deposit` do T52 (arredondamento a favor do cofre).
//!
//! Cota só nasce aqui: a autoridade de mint do DOM é o PDA do cofre (D15), então
//! não existe caminho fora do `deposit` que emita cota.

#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{
        constants::{MIN_DEPOSIT, NAV_GENESIS},
        error::DomError,
        events::Deposited,
    },
    solana_signer::Signer,
};

// ---------------------------------------------------------------------------
// T01 — depósito na gênese
// ---------------------------------------------------------------------------

/// NAV = 1,000000: cota sai 1:1 com o USDC aportado. O USDC vai para o caixa do
/// fundo, não fica no bolso do cotista.
#[test]
fn t01_deposito_na_genese_e_um_para_um() {
    let mut env = Env::new();
    assert_eq!(env.vault().nav, NAV_GENESIS, "NAV de genese");

    let cotista = env.carteira(1_000 * UNIT);
    let meta = env.deposit(&cotista, 1_000 * UNIT);

    assert_eq!(env.saldo_dom(&cotista), 1_000 * UNIT, "cotas emitidas");
    assert_eq!(env.supply_dom(), 1_000 * UNIT, "supply");
    assert_eq!(env.saldo_usdc(&cotista), 0, "USDC saiu do cotista");
    assert_eq!(
        saldo(&env.svm, &treasury_pda()),
        1_000 * UNIT,
        "USDC entrou no caixa"
    );
    assert!(tem_evento::<Deposited>(&meta), "evento Deposited emitido");
}

// ---------------------------------------------------------------------------
// T02 — depósito pós-valorização
// ---------------------------------------------------------------------------

/// Com NAV > 1 o novato compra menos cota pelo mesmo USDC — e o NAV **não muda**
/// depois do depósito. É o que impede o novato de diluir quem já estava dentro.
///
/// O NAV vem de `publish_nav` de verdade — o atalho de harness que existia
/// enquanto a instrução não existia foi removido.
#[test]
fn t02_deposito_pos_valorizacao_nao_dilui_antigos() {
    let mut env = Env::new();

    let antigo = env.cotista(1_000 * UNIT); // 1.000 cotas ao NAV 1,000000
    assert_eq!(env.saldo_dom(&antigo), 1_000 * UNIT);

    // Fundo valoriza 25%: NAV 1,250000.
    env.publish_nav(1_250_000);

    let novato = env.carteira(1_000 * UNIT);
    env.deposit(&novato, 1_000 * UNIT);

    // 1.000 USDC / 1,25 = 800 cotas.
    assert_eq!(env.saldo_dom(&novato), 800 * UNIT, "cotas do novato");
    assert_eq!(
        env.saldo_dom(&antigo),
        1_000 * UNIT,
        "cotas do antigo intactas"
    );
    assert_eq!(env.vault().nav, 1_250_000, "NAV inalterado pelo deposito");

    // O antigo continua valendo 1.250 USDC (1.000 cotas x 1,25) e o novato
    // 1.000 USDC (800 x 1,25) — ninguém transferiu valor para ninguém.
    assert_eq!(env.supply_dom(), 1_800 * UNIT);
}

// ---------------------------------------------------------------------------
// T03 / T04 — o mínimo é inclusivo
// ---------------------------------------------------------------------------

/// 199,999999 não entra; 200,000000 entra. O limite é inclusivo e a fronteira é
/// de uma unidade mínima — é onde erro de `>` contra `>=` se esconde.
#[test]
fn t03_t04_minimo_de_deposito_e_inclusivo() {
    let mut env = Env::new();
    let cotista = env.carteira(1_000 * UNIT);

    assert_dom_error(
        env.deposit_raw(&cotista, MIN_DEPOSIT - 1),
        DomError::DepositBelowMinimum,
        "T03 199,999999 USDC",
    );
    assert_eq!(env.saldo_dom(&cotista), 0, "T03 nao emitiu cota");

    env.deposit(&cotista, MIN_DEPOSIT);
    assert_eq!(
        env.saldo_dom(&cotista),
        MIN_DEPOSIT,
        "T04 200,000000 USDC entra"
    );
}

// ---------------------------------------------------------------------------
// T05 — carteira fora da whitelist
// ---------------------------------------------------------------------------

/// Mesmo caminho fail-closed do hook: conta de whitelist ausente é rejeição, não
/// omissão. E desabilitar depois volta a bloquear.
#[test]
fn t05_deposito_de_carteira_fora_da_whitelist_rejeita() {
    let mut env = Env::new();

    // `carteira` já habilita; aqui a habilitação é desfeita para testar os dois
    // estados com a mesma conta.
    let cotista = env.carteira(1_000 * UNIT);
    env.whitelist(&cotista.wallet.pubkey(), false);

    assert_dom_error(
        env.deposit_raw(&cotista, 500 * UNIT),
        DomError::NotWhitelisted,
        "T05 whitelist desativada",
    );

    // Contra-teste: habilitada, o mesmo depósito passa.
    env.whitelist(&cotista.wallet.pubkey(), true);
    env.deposit(&cotista, 500 * UNIT);
    assert_eq!(env.saldo_dom(&cotista), 500 * UNIT);
}

// ---------------------------------------------------------------------------
// T52 (metade do deposit) — arredondamento a favor do cofre
// ---------------------------------------------------------------------------

/// NAV 3,000000: 1.000 USDC dariam 333,333333... cotas. O cofre emite 333,333333
/// e fica com a fração. Arredondar para cima seria diluir quem já está dentro.
#[test]
fn t52_arredondamento_favorece_o_cofre() {
    let mut env = Env::new();
    env.publish_nav(3 * UNIT);

    let cotista = env.carteira(1_000 * UNIT);
    env.deposit(&cotista, 1_000 * UNIT);

    assert_eq!(env.saldo_dom(&cotista), 333_333_333, "cotas truncadas");
    // O cofre recebeu 1.000 USDC inteiros e emitiu cota de menos: a diferença
    // fica no patrimônio, nunca no bolso do usuário.
    assert_eq!(saldo(&env.svm, &treasury_pda()), 1_000 * UNIT);
}
