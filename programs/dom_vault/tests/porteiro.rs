//! **O porteiro da whitelist — `D-F2-34`.**
//!
//! Aprovar cotista era proposta 2/3 **por pessoa**. O gargalo nunca foi a
//! decisão — a mesa decide na fila do painel — e sim a assinatura de hardware
//! para executar o que já estava decidido. Com fila de espera, trava a captação.
//!
//! ⚠️ **Este arquivo existe mais para provar o que o porteiro NÃO pode.**
//!
//! Delegar aprovação de whitelist é reduzir controle de verdade, e a única coisa
//! que torna a redução aceitável é o tamanho exato do que foi delegado. Um
//! ensaio que só prove "o porteiro aprova" mede a parte fácil e deixa a cara
//! sem cobertura.

#![allow(clippy::result_large_err)]

mod common;

use {common::*, dom_vault::error::DomError, solana_keypair::Keypair, solana_signer::Signer};

/// Sem porteiro nomeado, nada muda: só a mesa aprova.
#[test]
fn t_sem_porteiro_so_a_mesa_aprova() {
    let mut env = Env::new();
    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), SOL).unwrap();
    let alvo = Keypair::new();

    assert_dom_error(
        env.update_whitelist_raw(&intruso, &alvo.pubkey(), true),
        DomError::Unauthorized,
        "conta de porteiro nem existe",
    );
}

/// Nomeado, o porteiro aprova sozinho — é o pedido.
#[test]
fn t_o_porteiro_aprova_sozinho() {
    let mut env = Env::new();
    let porteiro = Keypair::new();
    env.svm.airdrop(&porteiro.pubkey(), SOL).unwrap();

    assert_ok(
        env.set_whitelist_operator_raw(&env.authority.insecure_clone(), porteiro.pubkey()),
        "a mesa nomeia",
    );

    let c = env.carteira_sem_whitelist(1_000 * UNIT);
    assert_ok(
        env.update_whitelist_raw(&porteiro, &c.wallet.pubkey(), true),
        "o porteiro aprova, sem 2/3",
    );
    env.deposit(&c, 100 * UNIT);
    assert_eq!(
        env.saldo_dom(&c),
        100 * UNIT,
        "e o aprovado aporta de verdade"
    );
}

/// ⚠️ O ensaio central: o porteiro abre a porta e NADA MAIS.
#[test]
fn t_o_porteiro_nao_move_dinheiro_nem_muda_regra() {
    let mut env = Env::new();
    let porteiro = Keypair::new();
    env.svm.airdrop(&porteiro.pubkey(), SOL).unwrap();
    assert_ok(
        env.set_whitelist_operator_raw(&env.authority.insecure_clone(), porteiro.pubkey()),
        "a mesa nomeia",
    );

    // Não pausa.
    assert_dom_error(
        env.pause_raw(&porteiro),
        DomError::Unauthorized,
        "porteiro pausando",
    );

    // Não mexe no piso do cofre.
    assert_dom_error(
        env.update_min_deposit_raw(&porteiro, UNIT),
        DomError::Unauthorized,
        "porteiro mexendo no min_deposit",
    );

    // E não nomeia a si mesmo de novo — nem outro porteiro.
    let outro = Keypair::new();
    assert_dom_error(
        env.set_whitelist_operator_raw(&porteiro, outro.pubkey()),
        DomError::Unauthorized,
        "porteiro nomeando porteiro",
    );
}

/// ⚠️ O ENSAIO QUE DEFINE A DELEGACAO: o porteiro NAO desinscreve.
///
/// Desinscrever TRANCA o dinheiro de outro. Quem sai da whitelist nao transfere
/// cota — o hook barra as duas pontas — e nao pede resgate de capital, porque
/// `solicitar` exige whitelist. O capital fica imobilizado ate' a mesa readmitir
/// **por 2/3, que e' o quorum que a chave do porteiro contorna**.
///
/// Inscrever tem limite natural: a pessoa ainda precisa APORTAR. Desinscrever
/// nao tem limite nenhum, e e' unilateral.
#[test]
fn t_o_porteiro_nao_desinscreve() {
    let mut env = Env::new();
    let porteiro = Keypair::new();
    env.svm.airdrop(&porteiro.pubkey(), SOL).unwrap();
    assert_ok(
        env.set_whitelist_operator_raw(&env.authority.insecure_clone(), porteiro.pubkey()),
        "a mesa nomeia",
    );

    let c = env.carteira_sem_whitelist(1_000 * UNIT);
    assert_ok(
        env.update_whitelist_raw(&porteiro, &c.wallet.pubkey(), true),
        "inscrever, pode",
    );

    assert_dom_error(
        env.update_whitelist_raw(&porteiro, &c.wallet.pubkey(), false),
        DomError::Unauthorized,
        "DESINSCREVER, nao pode",
    );

    // E a carteira continua valendo: o aporte passa.
    env.deposit(&c, 100 * UNIT);
    assert_eq!(
        env.saldo_dom(&c),
        100 * UNIT,
        "a recusa nao deixou a ficha meio escrita"
    );

    // A mesa desinscreve normalmente.
    assert_ok(
        env.update_whitelist_raw(&env.authority.insecure_clone(), &c.wallet.pubkey(), false),
        "a mesa desinscreve",
    );
}

/// E a EXCECAO DE PISO tambem fica com a mesa — e' politica sobre dinheiro.
#[test]
fn t_o_porteiro_nao_concede_piso_menor() {
    let mut env = Env::new();
    let porteiro = Keypair::new();
    env.svm.airdrop(&porteiro.pubkey(), SOL).unwrap();
    assert_ok(
        env.set_whitelist_operator_raw(&env.authority.insecure_clone(), porteiro.pubkey()),
        "nomeia",
    );

    let c = Keypair::new();
    assert_dom_error(
        env.update_whitelist_com_piso_por(&porteiro, &c.pubkey(), true, UNIT),
        DomError::Unauthorized,
        "porteiro concedendo piso menor",
    );
    assert_ok(
        env.update_whitelist_raw(&porteiro, &c.pubkey(), true),
        "no piso do cofre, pode",
    );
}

/// Destituir é `Pubkey::default()`, e ele para de valer na hora.
#[test]
fn t_a_mesa_destitui_e_ele_para_na_hora() {
    let mut env = Env::new();
    let porteiro = Keypair::new();
    env.svm.airdrop(&porteiro.pubkey(), SOL).unwrap();
    assert_ok(
        env.set_whitelist_operator_raw(&env.authority.insecure_clone(), porteiro.pubkey()),
        "nomeia",
    );

    let a = Keypair::new();
    assert_ok(
        env.update_whitelist_raw(&porteiro, &a.pubkey(), true),
        "aprova enquanto vale",
    );

    assert_ok(
        env.set_whitelist_operator_raw(
            &env.authority.insecure_clone(),
            anchor_lang::prelude::Pubkey::default(),
        ),
        "destitui",
    );

    let b = Keypair::new();
    assert_dom_error(
        env.update_whitelist_raw(&porteiro, &b.pubkey(), true),
        DomError::Unauthorized,
        "destituido NAO aprova mais",
    );
}

/// ⚠️ `Pubkey::default()` é DESLIGADO, e nunca um assinante válido.
///
/// Sem esta trava, qualquer um que conseguisse assinar como a chave nula — ou um
/// caminho que a produzisse por engano — entraria como porteiro.
#[test]
fn t_a_chave_nula_nao_e_porteiro() {
    let mut env = Env::new();
    assert_ok(
        env.set_whitelist_operator_raw(
            &env.authority.insecure_clone(),
            anchor_lang::prelude::Pubkey::default(),
        ),
        "porteiro desligado",
    );
    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), SOL).unwrap();
    let alvo = Keypair::new();
    assert_dom_error(
        env.update_whitelist_raw(&intruso, &alvo.pubkey(), true),
        DomError::Unauthorized,
        "com porteiro nulo, ninguem alem da mesa entra",
    );
}

/// A mesa continua aprovando, com porteiro nomeado ou sem.
#[test]
fn t_a_mesa_nunca_perde_a_whitelist() {
    let mut env = Env::new();
    let porteiro = Keypair::new();
    env.svm.airdrop(&porteiro.pubkey(), SOL).unwrap();
    assert_ok(
        env.set_whitelist_operator_raw(&env.authority.insecure_clone(), porteiro.pubkey()),
        "nomeia",
    );
    let c = Keypair::new();
    assert_ok(
        env.update_whitelist_raw(&env.authority.insecure_clone(), &c.pubkey(), true),
        "a mesa aprova mesmo com porteiro nomeado",
    );
}
