//! **O piso de aporte POR CARTEIRA — Upgrade G, `D-F2-30`.**
//!
//! O mínimo do cofre continua sendo o padrão. O que entrou é uma **exceção por
//! carteira**, gravada na própria entrada de whitelist, que o `deposit` já
//! carregava — nenhuma conta nova, nenhuma leitura a mais.
//!
//! `0` significa "sem exceção". É o que faz a entrada anterior ao upgrade, de
//! 42 bytes, continuar valendo sem ser tocada — e é isso que o `t_layout_antigo`
//! prova, porque é a diferença entre um upgrade e um apagão.

#![allow(clippy::result_large_err)]

mod common;

use {common::*, dom_vault::error::DomError, solana_signer::Signer};

/// Sem exceção, vale o piso do cofre — o comportamento de sempre.
#[test]
fn t_sem_excecao_vale_o_piso_do_cofre() {
    let mut env = Env::new();
    let c = env.carteira(1_000 * UNIT);

    assert_dom_error(
        env.deposit_raw(&c, UNIT),
        DomError::DepositBelowMinimum,
        "1 USDC sem excecao",
    );
    env.deposit(&c, 100 * UNIT);
    assert_eq!(env.saldo_dom(&c), 100 * UNIT);
}

/// Com exceção, a carteira aporta abaixo do piso do cofre.
#[test]
fn t_com_excecao_aporta_abaixo_do_piso() {
    let mut env = Env::new();
    let c = env.carteira(1_000 * UNIT);
    assert_ok(
        env.update_whitelist_com_piso(&c.wallet.pubkey(), true, UNIT),
        "piso proprio de 1 USDC",
    );

    env.deposit(&c, UNIT);
    assert_eq!(env.saldo_dom(&c), UNIT, "a cota nasceu com 1 USDC");
}

/// A exceção é **desta** carteira. A do lado continua no piso do cofre.
#[test]
fn t_a_excecao_nao_vaza_para_a_carteira_ao_lado() {
    let mut env = Env::new();
    let privilegiada = env.carteira(1_000 * UNIT);
    let comum = env.carteira(1_000 * UNIT);
    assert_ok(
        env.update_whitelist_com_piso(&privilegiada.wallet.pubkey(), true, UNIT),
        "piso proprio",
    );

    env.deposit(&privilegiada, UNIT);
    assert_dom_error(
        env.deposit_raw(&comum, UNIT),
        DomError::DepositBelowMinimum,
        "a carteira ao lado NAO herda a excecao",
    );
}

/// A exceção sai voltando o campo para `0`.
#[test]
fn t_a_excecao_se_desfaz() {
    let mut env = Env::new();
    let c = env.carteira(1_000 * UNIT);
    assert_ok(
        env.update_whitelist_com_piso(&c.wallet.pubkey(), true, UNIT),
        "liga a excecao",
    );
    env.deposit(&c, UNIT);

    assert_ok(
        env.update_whitelist_com_piso(&c.wallet.pubkey(), true, 0),
        "desliga a excecao",
    );

    // -----------------------------------------------------------------------
    // ⚠️ A CONFERENCIA E' COM CARTEIRA NOVA — `D-F2-33`.
    //
    // `c` ja' aportou acima, entao ela e' cotista e o piso nao se aplica mais a
    // ela: o piso e' DE ENTRADA. Conferir nela provaria o comportamento antigo.
    //
    // O que este ensaio afirma continua sendo o mesmo: desligar a excecao
    // devolve a carteira ao piso do cofre. So' que isso se mede em quem esta'
    // ENTRANDO — e a outra metade, que `c` passa a nao ser barrada, e' afirmada
    // logo abaixo para que reverter qualquer um dos dois lados quebre aqui.
    // -----------------------------------------------------------------------
    let nova = env.carteira(1_000 * UNIT);
    assert_ok(
        env.update_whitelist_com_piso(&nova.wallet.pubkey(), true, 0),
        "a nova entra sem excecao",
    );
    assert_dom_error(
        env.deposit_raw(&nova, UNIT),
        DomError::DepositBelowMinimum,
        "sem excecao, quem ENTRA volta ao piso do cofre",
    );

    assert_ok(
        env.deposit_raw(&c, UNIT),
        "e quem ja' e' cotista reaporta sem piso, com ou sem excecao",
    );
}

/// ---------------------------------------------------------------------------
/// **O ENSAIO QUE IMPORTA: a entrada de 42 bytes continua valendo.**
/// ---------------------------------------------------------------------------
/// Toda carteira aprovada antes do Upgrade G tem uma entrada de 42 bytes, sem o
/// campo novo. O `require_whitelisted` é a MESMA função do `deposit` e do hook
/// de transferência.
///
/// Se ela falhasse nessas entradas, no instante do upgrade **todo aporte e toda
/// transferência de cota parariam**, e só voltariam depois de uma migração conta
/// a conta — com o fundo fechado no meio.
///
/// Este ensaio planta a conta no layout antigo, na marra, e exige que ela
/// continue funcionando e signifique "sem exceção".
#[test]
fn t_layout_antigo_continua_valendo_e_significa_sem_excecao() {
    let mut env = Env::new();
    let c = env.carteira(1_000 * UNIT);

    // A entrada como ela existia ANTES do campo entrar.
    env.plantar_whitelist_antiga(&c.wallet.pubkey(), true);

    // Aporta normalmente, ao piso do cofre.
    env.deposit(&c, 100 * UNIT);
    assert_eq!(env.saldo_dom(&c), 100 * UNIT, "a entrada velha vale");

    // -----------------------------------------------------------------------
    // E o campo ausente lê como "sem exceção", não como "piso zero".
    //
    // ⚠️ Medido em carteira NOVA com entrada antiga — `c` ja' aportou acima e o
    // piso e' DE ENTRADA (`D-F2-33`). O que se afirma aqui e' sobre o LAYOUT, e
    // o layout so' se observa em quem ainda nao entrou.
    // -----------------------------------------------------------------------
    let nova = env.carteira(1_000 * UNIT);
    env.plantar_whitelist_antiga(&nova.wallet.pubkey(), true);
    assert_dom_error(
        env.deposit_raw(&nova, UNIT),
        DomError::DepositBelowMinimum,
        "campo ausente NAO libera aporte de 1 USDC",
    );
}

/// E a entrada velha **cresce** quando a mesa a toca, sem perder o que tinha.
#[test]
fn t_a_entrada_velha_cresce_quando_tocada() {
    let mut env = Env::new();
    let c = env.carteira(1_000 * UNIT);
    env.plantar_whitelist_antiga(&c.wallet.pubkey(), true);

    assert_ok(
        env.update_whitelist_com_piso(&c.wallet.pubkey(), true, UNIT),
        "a mesa toca a entrada antiga",
    );
    env.deposit(&c, UNIT);
    assert_eq!(env.saldo_dom(&c), UNIT, "cresceu e guardou a excecao");
}

// ===========================================================================
// `deposit_para` — o bônus numa assinatura. Upgrade G.
// ===========================================================================

/// O caminho feliz: a mesa paga, o membro recebe a cota.
#[test]
fn t_bonus_a_mesa_paga_o_membro_recebe() {
    let mut env = Env::new();
    let mesa = env.carteira(1_000 * UNIT);
    let membro = env.carteira(0);

    assert_ok(
        env.deposit_para_raw(&mesa, &membro, 100 * UNIT),
        "bonus de 100 USDC",
    );
    assert_eq!(
        env.saldo_dom(&membro),
        100 * UNIT,
        "a cota foi para o membro"
    );
    assert_eq!(env.saldo_dom(&mesa), 0, "quem pagou NAO recebe cota");
    assert_eq!(
        env.saldo_usdc(&mesa),
        900 * UNIT,
        "o USDC saiu de quem pagou"
    );
}

/// **A trava do cabeçalho.** `mint_to` não dispara o hook — sem esta
/// conferência, isto cunharia cota em qualquer carteira do mundo.
#[test]
fn t_bonus_recusa_beneficiario_fora_da_whitelist() {
    let mut env = Env::new();
    let mesa = env.carteira(1_000 * UNIT);
    let forasteiro = env.carteira_sem_whitelist(0);

    assert_dom_error(
        env.deposit_para_raw(&mesa, &forasteiro, 100 * UNIT),
        DomError::NotWhitelisted,
        "beneficiario fora da whitelist",
    );
}

/// A outra ponta também: dinheiro de carteira que a mesa não aprovou é canal
/// que ela não abriu, mesmo que a cota vá para alguém aprovado.
#[test]
fn t_bonus_recusa_pagador_fora_da_whitelist() {
    let mut env = Env::new();
    let estranho = env.carteira_sem_whitelist(1_000 * UNIT);
    let membro = env.carteira(0);

    assert_dom_error(
        env.deposit_para_raw(&estranho, &membro, 100 * UNIT),
        DomError::NotWhitelisted,
        "pagador fora da whitelist",
    );
}

/// O piso é o DO BENEFICIÁRIO, não o de quem paga.
#[test]
fn t_bonus_usa_o_piso_do_beneficiario() {
    let mut env = Env::new();
    let mesa = env.carteira(1_000 * UNIT);
    let membro = env.carteira(0);
    assert_ok(
        env.update_whitelist_com_piso(&membro.wallet.pubkey(), true, UNIT),
        "excecao no BENEFICIARIO",
    );

    assert_ok(
        env.deposit_para_raw(&mesa, &membro, UNIT),
        "1 USDC passa pelo piso do beneficiario",
    );
    assert_eq!(env.saldo_dom(&membro), UNIT);
}

// ===========================================================================
// ⚠️ O PIOR CASO DO UPGRADE G: o HOOK contra entradas de 42 bytes.
// ===========================================================================
// `require_whitelisted` é a mesma função do `deposit` E do hook de
// transferência. Os ensaios de hook que já existiam usam entradas criadas pelo
// binário NOVO, de 50 bytes — nenhum exercitava o layout antigo.
//
// É o vão que importa: se a leitura falhasse nas entradas de 42 bytes, no
// segundo em que o upgrade subisse **toda transferência de cota pararia**, e
// não haveria conserto sem migrar conta a conta com o fundo fechado.

/// Transferência de cota entre DUAS carteiras com entrada de 42 bytes.
#[test]
fn t_o_hook_aceita_as_duas_pontas_no_layout_antigo() {
    let mut env = Env::new();
    let a = env.cotista(1_000 * UNIT);
    let b = env.carteira(0);

    // Depois de terem cota e conta, as duas entradas voltam ao layout antigo.
    env.plantar_whitelist_antiga(&a.wallet.pubkey(), true);
    env.plantar_whitelist_antiga(&b.wallet.pubkey(), true);

    assert_ok(
        env.transfer(&a, &b.dom, &b.wallet.pubkey(), 100 * UNIT),
        "transferencia de cota com AS DUAS entradas em 42 bytes",
    );
    assert_eq!(env.saldo_dom(&b), 100 * UNIT, "a cota chegou");
}

/// E o hook continua FECHANDO contra quem não está aprovado, mesmo com o
/// layout antigo do outro lado — a compatibilidade não pode virar frouxidão.
#[test]
fn t_o_hook_recusa_destino_nao_aprovado_com_layout_antigo() {
    let mut env = Env::new();
    let a = env.cotista(1_000 * UNIT);
    let forasteiro = env.carteira_sem_whitelist(0);
    env.plantar_whitelist_antiga(&a.wallet.pubkey(), true);

    assert!(
        env.transfer(&a, &forasteiro.dom, &forasteiro.wallet.pubkey(), 100 * UNIT)
            .is_err(),
        "entrada antiga na origem NAO afrouxa a trava do destino",
    );
}

/// E uma entrada antiga DESATIVADA continua barrando.
#[test]
fn t_entrada_antiga_desativada_continua_barrando() {
    let mut env = Env::new();
    let a = env.cotista(1_000 * UNIT);
    let b = env.carteira(0);
    env.plantar_whitelist_antiga(&a.wallet.pubkey(), false); // active = false

    assert!(
        env.transfer(&a, &b.dom, &b.wallet.pubkey(), 100 * UNIT)
            .is_err(),
        "layout antigo com active=false continua barrando",
    );
}
