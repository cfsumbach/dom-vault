//! **O piso é DE ENTRADA — `D-F2-33`.**
//!
//! O `min_deposit` dimensiona QUEM ENTRA: é o tamanho mínimo de uma posição
//! nova. Aplicá-lo de novo a quem já é cotista produzia o absurdo de o fundo
//! **recusar dinheiro de quem já está dentro**.
//!
//! O Upgrade G entregou `min_deposit_proprio` — exceção POR CARTEIRA, uma
//! proposta 2/3 para cada investidor. Aquilo resolvia a mesa conceder piso menor
//! a alguém; não resolvia o reaporte, que é automático por natureza e não
//! deveria custar voto nenhum.
//!
//! ⚠️ O ensaio que importa é o `t_a_leitura_e_antes_da_emissao`: o saldo tem de
//! ser o ANTERIOR ao `mint_to`. Lido depois, todo aporte seria de cotista e o
//! piso não valeria para ninguém — o defeito na direção que deixa entrar, que é
//! a cara.

#![allow(clippy::result_large_err)]

mod common;

use {common::*, dom_vault::error::DomError};

/// O primeiro aporte obedece o piso. O segundo não tem piso.
#[test]
fn t_o_segundo_aporte_nao_tem_piso() {
    let mut env = Env::new();
    let c = env.carteira(1_000 * UNIT);

    // Entra: tem de pagar o piso.
    assert_dom_error(
        env.deposit_raw(&c, UNIT),
        DomError::DepositBelowMinimum,
        "1 USDC na ENTRADA",
    );
    env.deposit(&c, 100 * UNIT);

    // Já é cotista: 1 USDC passa.
    assert_ok(env.deposit_raw(&c, UNIT), "1 USDC no REAPORTE");
    assert_eq!(env.saldo_dom(&c), 101 * UNIT);
}

/// ⚠️ O ensaio central: se a leitura fosse DEPOIS do `mint_to`, este passaria
/// — e o piso deixaria de existir para todo mundo, silenciosamente.
#[test]
fn t_a_leitura_e_antes_da_emissao() {
    let mut env = Env::new();
    let c = env.carteira(1_000 * UNIT);

    // Saldo zero. Se o handler lesse o saldo pós-emissão, `ja_e_cotista` seria
    // verdadeiro já na primeira vez e este aporte passaria.
    assert_eq!(env.saldo_dom(&c), 0);
    assert_dom_error(
        env.deposit_raw(&c, UNIT),
        DomError::DepositBelowMinimum,
        "a PRIMEIRA vez tem de reprovar",
    );
    assert_eq!(env.saldo_dom(&c), 0, "e nada foi emitido");
}

/// Ser cotista não é transitivo: a carteira ao lado continua no piso.
#[test]
fn t_a_isencao_nao_vaza_para_a_carteira_ao_lado() {
    let mut env = Env::new();
    let dentro = env.carteira(1_000 * UNIT);
    let nova = env.carteira(1_000 * UNIT);

    env.deposit(&dentro, 100 * UNIT);
    assert_ok(env.deposit_raw(&dentro, UNIT), "quem esta dentro reaporta");

    assert_dom_error(
        env.deposit_raw(&nova, UNIT),
        DomError::DepositBelowMinimum,
        "a de fora continua pagando o piso",
    );
}

/// Poeira continua barrada: sem piso, a trava passa a ser `ZeroShares`.
#[test]
fn t_o_reaporte_nao_abre_a_porta_para_poeira() {
    let mut env = Env::new();
    let c = env.carteira(1_000 * UNIT);
    env.deposit(&c, 100 * UNIT);

    // Zero é recusado, e não por piso.
    assert_dom_error(
        env.deposit_raw(&c, 0),
        DomError::ZeroShares,
        "aporte de zero",
    );
    assert_eq!(env.saldo_dom(&c), 100 * UNIT);
}

/// Sair de tudo volta a ser entrada: quem zera a posição paga o piso de novo.
#[test]
fn t_quem_zera_a_posicao_volta_a_pagar_o_piso() {
    let mut env = Env::new();
    let c = env.carteira(1_000 * UNIT);
    env.deposit(&c, 100 * UNIT);
    let tudo = env.saldo_dom(&c);
    assert_ok(env.burn(&c, tudo), "zera a posicao");
    assert_eq!(env.saldo_dom(&c), 0);

    assert_dom_error(
        env.deposit_raw(&c, UNIT),
        DomError::DepositBelowMinimum,
        "posicao zerada = entrada nova",
    );
}

/// O bônus olha o saldo do BENEFICIÁRIO, nunca o do pagador.
#[test]
fn t_o_bonus_olha_o_beneficiario_e_nao_o_pagador() {
    let mut env = Env::new();
    let pagador = env.carteira(1_000 * UNIT);
    let novo = env.carteira(1_000 * UNIT);

    // O pagador é cotista; o beneficiário não é.
    env.deposit(&pagador, 100 * UNIT);

    assert_dom_error(
        env.deposit_para_raw(&pagador, &novo, UNIT),
        DomError::DepositBelowMinimum,
        "pagador cotista NAO isenta beneficiario novo",
    );

    // E quando o beneficiário já é cotista, passa.
    env.deposit(&novo, 100 * UNIT);
    assert_ok(
        env.deposit_para_raw(&pagador, &novo, UNIT),
        "beneficiario ja cotista",
    );
}
