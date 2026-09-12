//! **Resgate de capital — D+180 como teto, pago pelo pior NAV do período.**
//!
//! Sucede a fila D+30, que saiu inteira: `request_redeem`, `process_redemptions`,
//! a `RedeemQueue`, o cap por ciclo e o passivo congelado. Um produto, não dois.
//!
//! Cada trava tem contra-teste ao lado. As que mais importam são as três do
//! pior-NAV, porque é o único número que a mesa informa e o contrato não
//! consegue derivar — e três limites por cima é tudo o que dá para exigir dele.

#![allow(clippy::result_large_err)]

mod common;

use {
    anchor_lang::prelude::Pubkey,
    common::*,
    dom_vault::{
        constants::{MIN_RESGATE_CAPITAL_USDC, NAV_SCALE, RESGATE_CAPITAL_PRAZO},
        error::DomError,
    },
    solana_signer::Signer,
};

const APORTE: u64 = 20_000 * UNIT;

/// Cofre com um cotista de 20.000 cotas e o endereço de resgate já nomeado e
/// abastecido pela mesa — que é o estado normal de operação.
fn cofre_pronto_para_resgatar(caixa_de_resgate: u64) -> (Env, Holder, Pubkey) {
    let mut env = Env::new();
    let cotista = env.cotista(APORTE);
    let pagadora = env.caixa_da_autoridade(caixa_de_resgate);
    env.update_endereco_resgate(pagadora);
    (env, cotista, pagadora)
}

/// O mesmo, com uma carteira operacional na allowlist — para os testes que
/// precisam do `deploy_capital`.
fn cofre_com_ops_e_resgate(caixa_de_resgate: u64) -> (Env, Holder, Holder) {
    let mut env = Env::new();
    let cotista = env.cotista(APORTE);
    let ops = env.carteira(0);
    env.update_deploy_allowlist(0, ops.wallet.pubkey());
    let pagadora = env.caixa_da_autoridade(caixa_de_resgate);
    env.update_endereco_resgate(pagadora);
    (env, cotista, ops)
}

// ---------------------------------------------------------------------------
// Solicitar
// ---------------------------------------------------------------------------

/// As cotas travam **saindo** da carteira. É isso que faz "não transfere a
/// fração travada" e "não pede de novo sobre a mesma cota" serem verdade sem
/// trava extra: elas não estão mais lá.
#[test]
fn solicitar_move_as_cotas_para_o_escrow_e_abre_o_pedido() {
    let (mut env, cotista, _) = cofre_pronto_para_resgatar(0);
    let antes = env.saldo_dom(&cotista);

    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);

    assert_eq!(id, 0, "o primeiro pedido leva o id 0");
    assert_eq!(env.saldo_dom(&cotista), antes - 5_000 * UNIT);
    assert_eq!(env.escrow_cotas(), 5_000 * UNIT);
    assert_eq!(env.vault().cotas_travadas_resgate, 5_000 * UNIT);
    assert_eq!(env.vault().proximo_pedido_id, 1);

    let p = env.pedido(id);
    assert_eq!(p.owner, cotista.wallet.pubkey());
    assert_eq!(p.cotas, 5_000 * UNIT);
    assert_eq!(p.nav_na_solicitacao, env.vault().nav);
    assert_eq!(p.pior_nav_visto, env.vault().nav, "a catraca nasce no teto");
    assert_eq!(p.vence_ts, p.solicitado_ts + RESGATE_CAPITAL_PRAZO);
    assert_eq!(p.estado, 0, "aberto");
}

/// Parcial: dois pedidos da mesma carteira, cada um com id, data e catraca
/// próprios. Era proibido na fila D+30 (D-F2-13) e é permitido aqui — não há
/// recurso comum a esgotar.
#[test]
fn parcial_abre_dois_pedidos_independentes_da_mesma_carteira() {
    let (mut env, cotista, _) = cofre_pronto_para_resgatar(0);

    let a = env.solicitar_resgate(&cotista, 5_000 * UNIT);
    env.avancar(3_600);
    let b = env.solicitar_resgate(&cotista, 6_000 * UNIT);

    assert_ne!(a, b);
    assert_eq!(env.pedido(a).cotas, 5_000 * UNIT);
    assert_eq!(env.pedido(b).cotas, 6_000 * UNIT);
    assert!(
        env.pedido(b).solicitado_ts > env.pedido(a).solicitado_ts,
        "cada pedido tem a sua data"
    );
    assert_eq!(env.vault().cotas_travadas_resgate, 11_000 * UNIT);
    assert_eq!(env.escrow_cotas(), 11_000 * UNIT);
}

/// Contra-teste do mínimo. Em **cotas**, não em USDC — a D-F2-03 vale: pedido de
/// resgate não fica refém da frescura do oráculo.
#[test]
fn abaixo_do_minimo_recusa_e_no_minimo_exato_passa() {
    let (mut env, cotista, _) = cofre_pronto_para_resgatar(0);

    assert_dom_error(
        env.solicitar_resgate_raw(&cotista, MIN_RESGATE_CAPITAL_USDC - 1),
        DomError::ResgateAbaixoDoMinimo,
        "um micro abaixo do minimo",
    );
    assert_ok(
        env.solicitar_resgate_raw(&cotista, MIN_RESGATE_CAPITAL_USDC),
        "o minimo exato entra",
    );
}

/// **NAV velho NÃO impede pedir.** É a D-F2-03 sendo provada, não só citada: o
/// `solicitar_resgate_capital` não chama `require_nav_fresco`.
#[test]
fn nav_vencido_nao_impede_o_pedido() {
    let (mut env, cotista, _) = cofre_pronto_para_resgatar(0);
    env.avancar_sem_oraculo(27 * 60 * 60);

    assert_ok(
        env.solicitar_resgate_raw(&cotista, 5_000 * UNIT),
        "pedido de resgate e' direito do cotista — nao fica refem do backend",
    );
}

#[test]
fn carteira_fora_da_whitelist_nao_pede() {
    let (mut env, _, _) = cofre_pronto_para_resgatar(0);
    let estranho = env.cotista(10_000 * UNIT);
    let authority = env.authority.insecure_clone();
    assert_ok(
        env.update_whitelist_raw(&authority, &estranho.wallet.pubkey(), false),
        "desativar a whitelist",
    );

    assert_dom_error(
        env.solicitar_resgate_raw(&estranho, 5_000 * UNIT),
        DomError::NotWhitelisted,
        "fora da whitelist",
    );
}

/// Pedir mais do que se tem morre na **transferência**, não no programa: no
/// resgate composto a cota sai antes de o pedido ser conferido, e o Token-2022
/// recusa saldo insuficiente. É a ordem certa — o programa nunca vê um pedido
/// sem lastro.
#[test]
fn sem_cotas_suficientes_recusa() {
    let (mut env, cotista, _) = cofre_pronto_para_resgatar(0);
    assert!(
        env.solicitar_resgate_raw(&cotista, APORTE + 1).is_err(),
        "pediu mais do que tem"
    );
    assert_eq!(env.escrow_cotas(), 0, "nada travou");
    assert_eq!(env.vault().proximo_pedido_id, 0, "nenhum pedido abriu");
}

#[test]
fn cofre_pausado_fecha_o_pedido() {
    let (mut env, cotista, _) = cofre_pronto_para_resgatar(0);
    env.pause();
    assert_dom_error(
        env.solicitar_resgate_raw(&cotista, 5_000 * UNIT),
        DomError::Paused,
        "pausado",
    );
}

// ---------------------------------------------------------------------------
// A catraca
// ---------------------------------------------------------------------------

/// Só desce. Chamar com NAV alto não faz nada — é o que torna seguro deixar a
/// instrução aberta a qualquer um.
#[test]
fn catraca_so_desce_e_qualquer_um_carimba() {
    let (mut env, cotista, _) = cofre_pronto_para_resgatar(0);
    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);
    let teto = env.pedido(id).pior_nav_visto;

    env.avancar(2 * 60 * 60);
    env.publish_nav(teto * 9 / 10);
    assert_ok(
        env.carimbar_nav_raw(id),
        "carimbar nao exige assinante nenhum",
    );
    let baixo = env.pedido(id).pior_nav_visto;
    assert!(baixo < teto, "desceu: {baixo} < {teto}");

    env.avancar(2 * 60 * 60);
    env.publish_nav(teto);
    env.carimbar_nav(id);
    assert_eq!(
        env.pedido(id).pior_nav_visto,
        baixo,
        "NAV alto nao levanta a catraca"
    );
}

// ---------------------------------------------------------------------------
// Efetivar
// ---------------------------------------------------------------------------

#[test]
fn efetivar_queima_a_cota_e_paga_do_endereco_de_resgate() {
    let (mut env, cotista, pagadora) = cofre_pronto_para_resgatar(10_000 * UNIT);
    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);

    let supply_antes = env.supply_dom();
    let caixa_do_cofre_antes = env.caixa();
    let usdc_antes = env.saldo_usdc(&cotista);
    let pior = env.vault().nav;

    env.efetivar_resgate(id, pior, cotista.usdc);

    let esperado = 5_000 * UNIT * pior / NAV_SCALE;
    assert_eq!(env.saldo_usdc(&cotista), usdc_antes + esperado);
    assert_eq!(
        env.supply_dom(),
        supply_antes - 5_000 * UNIT,
        "a cota morre, nao volta"
    );
    assert_eq!(env.escrow_cotas(), 0);
    assert_eq!(env.vault().cotas_travadas_resgate, 0);
    assert_eq!(env.pedido(id).estado, 1, "pago");
    assert_eq!(
        env.caixa(),
        caixa_do_cofre_antes,
        "o treasury NAO e' fonte do resgate de capital"
    );
    assert_eq!(env.saldo_usdc_da_conta(pagadora), 10_000 * UNIT - esperado);
}

/// **Não exige que o prazo tenha vencido.** D+180 é teto, não carência — a mesa
/// antecipa quando a saúde da posição permitir, e o critério é dela.
#[test]
fn efetivar_antes_do_prazo_e_permitido() {
    let (mut env, cotista, _) = cofre_pronto_para_resgatar(10_000 * UNIT);
    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);
    let nav = env.vault().nav;
    assert_ok(
        env.efetivar_resgate_raw(&env.authority.insecure_clone(), id, nav, cotista.usdc),
        "antecipar e' decisao da mesa, nao trava do contrato",
    );
}

/// As três travas do pior-NAV, uma a uma. Elas limitam **por cima** — protegem
/// quem fica de ver o fundo diluído por um resgate generoso. Quem sai é
/// protegido por verificabilidade contra o log, não por elas.
#[test]
fn pior_nav_e_limitado_por_cima_de_tres_maneiras() {
    let (mut env, cotista, _) = cofre_pronto_para_resgatar(50_000 * UNIT);
    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);
    let teto = env.pedido(id).nav_na_solicitacao;
    let authority = env.authority.insecure_clone();

    // 1. acima do NAV do pedido
    assert_dom_error(
        env.efetivar_resgate_raw(&authority, id, teto + 1, cotista.usdc),
        DomError::PiorNavAcimaDoTeto,
        "acima do nav_na_solicitacao",
    );

    // 2. acima da catraca
    env.avancar(2 * 60 * 60);
    env.publish_nav(teto * 9 / 10);
    env.carimbar_nav(id);
    assert_dom_error(
        env.efetivar_resgate_raw(&authority, id, teto, cotista.usdc),
        DomError::PiorNavAcimaDoTeto,
        "acima do pior_nav_visto",
    );

    // 3. zero
    assert_dom_error(
        env.efetivar_resgate_raw(&authority, id, 0, cotista.usdc),
        DomError::PiorNavZero,
        "zero",
    );

    // e o caminho feliz, pela catraca
    let catraca = env.pedido(id).pior_nav_visto;
    assert_ok(
        env.efetivar_resgate_raw(&authority, id, catraca, cotista.usdc),
        "o pior NAV carimbado paga",
    );
}

#[test]
fn sem_saldo_no_endereco_de_resgate_recusa() {
    let (mut env, cotista, _) = cofre_pronto_para_resgatar(100 * UNIT);
    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);
    let nav = env.vault().nav;
    assert_dom_error(
        env.efetivar_resgate_raw(&env.authority.insecure_clone(), id, nav, cotista.usdc),
        DomError::SemSaldoNoEnderecoDeResgate,
        "a mesa nao abasteceu — marca a data e espera",
    );
}

#[test]
fn nao_autoridade_nao_efetiva_e_nao_paga_duas_vezes() {
    let (mut env, cotista, _) = cofre_pronto_para_resgatar(50_000 * UNIT);
    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);
    let nav = env.vault().nav;

    let intruso = env.carteira(0);
    assert_dom_error(
        env.efetivar_resgate_raw(&intruso.wallet, id, nav, cotista.usdc),
        DomError::Unauthorized,
        "intruso",
    );

    env.efetivar_resgate(id, nav, cotista.usdc);
    assert_dom_error(
        env.efetivar_resgate_raw(&env.authority.insecure_clone(), id, nav, cotista.usdc),
        DomError::ResgateJaPago,
        "pagar duas vezes",
    );
}

// ---------------------------------------------------------------------------
// Vencimento — o dente da obrigação firme
// ---------------------------------------------------------------------------

/// **Enquanto houver resgate vencido não pago, o cofre não manda capital para
/// campo.** É o que converte a promessa de prazo em prioridade executável.
#[test]
fn resgate_vencido_trava_o_deploy_capital_e_o_pagamento_destrava() {
    let (mut env, cotista, ops) = cofre_com_ops_e_resgate(50_000 * UNIT);
    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);

    // antes do vencimento não se marca
    assert_dom_error(
        env.marcar_vencido_raw(id),
        DomError::ResgateNaoVenceu,
        "ainda no prazo",
    );

    env.avancar(RESGATE_CAPITAL_PRAZO + 1);

    // permissionless: quem marca é o prejudicado
    assert_ok(
        env.marcar_vencido_raw(id),
        "o proprio investidor aciona, sem depender de ninguem",
    );
    assert_eq!(env.pedido(id).estado, 2, "em atraso");
    assert_eq!(env.vault().resgates_em_atraso, 1);

    assert_dom_error(
        env.marcar_vencido_raw(id),
        DomError::ResgateJaEmAtraso,
        "marcar duas vezes",
    );

    assert_dom_error(
        env.deploy_capital_raw(&env.authority.insecure_clone(), ops.usdc, 1_000 * UNIT),
        DomError::ResgatesEmAtraso,
        "nao se aumenta a aposta devendo a quem pediu para sair",
    );

    let nav = env.vault().nav;
    env.efetivar_resgate(id, nav, cotista.usdc);
    assert_eq!(env.vault().resgates_em_atraso, 0);
    assert_ok(
        env.deploy_capital_raw(&env.authority.insecure_clone(), ops.usdc, 1_000 * UNIT),
        "pago o atraso, o capital volta a poder sair",
    );
}

// ---------------------------------------------------------------------------
// O endereço de resgate
// ---------------------------------------------------------------------------

#[test]
fn endereco_de_resgate_nasce_vazio_e_so_a_mesa_nomeia() {
    let mut env = Env::new();
    assert_eq!(
        env.vault().endereco_resgate,
        Pubkey::default(),
        "nasce vazio, como a allowlist"
    );

    let pagadora = env.caixa_da_autoridade(1_000 * UNIT);
    let intruso = env.carteira(0);
    assert_dom_error(
        env.update_endereco_resgate_raw(&intruso.wallet, pagadora),
        DomError::Unauthorized,
        "intruso nomeando a conta pagadora",
    );

    env.update_endereco_resgate(pagadora);
    assert_eq!(env.vault().endereco_resgate, pagadora);
}

/// A conta pagadora tem de ser do Vault do Squads — mesmo quórum que aprova.
#[test]
fn endereco_de_resgate_de_terceiro_recusa() {
    let mut env = Env::new();
    let estranho = env.carteira(1_000 * UNIT);
    let resultado = env.update_endereco_resgate_raw(&env.authority.insecure_clone(), estranho.usdc);
    assert!(
        resultado.is_err(),
        "conta cuja autoridade nao e' a mesa nao vira endereco de resgate"
    );
}
