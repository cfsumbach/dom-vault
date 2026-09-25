//! Upgrade K (D-F2-45) — **ninguém entra abaixo do piso**.
//!
//! O aporte cunha ao `max(nav, nav_piso)`. O `publish_nav` continua publicando
//! o bruto: drawdown é real e aparece na tela. A trava é na emissão, que é o
//! único lugar onde a diluição acontece.
//!
//! O caso que a mesa decidiu evitar aconteceu em 23/09/2026: uma perna sem
//! leitor derrubou o NAV publicado para 0,909130 e quem aportasse naquela
//! janela levaria 16,4 % mais cotas de graça.
#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{error::DomError, events::Deposited},
    solana_signer::Signer,
};

/// Deixa o cofre com NAV publicado ABAIXO do piso: o capital vai a campo (o
/// piso o conta ao custo) e o oráculo publica uma marcação menor.
fn cofre_em_drawdown() -> (Env, Holder) {
    let mut env = Env::new();
    let primeiro = env.cotista(10_000 * UNIT);
    let ops = env.carteira(0);
    env.update_deploy_allowlist(0, ops.wallet.pubkey());
    env.deploy_capital(ops.usdc, 9_000 * UNIT); // a reserva de 10% fica no caixa
                                                /* O piso só é recalculado quando o NAV é publicado. */
    env.publish_nav(1_000_000);
    assert_eq!(
        env.vault().nav_piso,
        1_000_000,
        "piso = (capital em campo + caixa) ÷ supply — a reserva nao some da conta"
    );
    /* Marcação 10 % abaixo: o bruto cai, o piso não. */
    env.publish_nav(900_000);
    let v = env.vault();
    assert_eq!(v.nav, 900_000);
    assert_eq!(
        v.nav_piso, 1_000_000,
        "o piso e' capital ao custo: nao cai com a marcacao"
    );
    (env, primeiro)
}

#[test]
fn aporte_com_bruto_abaixo_do_piso_cunha_pelo_piso() {
    let (mut env, _) = cofre_em_drawdown();
    let novo = env.carteira(1_000 * UNIT);
    env.whitelist(&novo.wallet.pubkey(), true);

    let meta = env.deposit(&novo, 1_000 * UNIT);
    let e: Deposited = evento(&meta).expect("Deposited");

    assert_eq!(
        e.nav, 900_000,
        "o evento continua dizendo o bruto publicado"
    );
    assert_eq!(e.preco_usado, 1_000_000, "cunhou ao PISO");
    assert!(e.pelo_piso, "e diz que foi pelo piso");
    assert_eq!(
        e.shares,
        1_000 * UNIT,
        "1.000 USDC ÷ 1,000000 = 1.000 cotas"
    );
    /* Ao bruto teria levado 1.111,11 cotas — 11,1 % a mais, tiradas de quem já estava. */
    assert_eq!(saldo(&env.svm, &novo.dom), 1_000 * UNIT);
}

#[test]
fn aporte_com_bruto_acima_do_piso_cunha_pelo_bruto() {
    let mut env = Env::new();
    let _ = env.cotista(10_000 * UNIT);
    let ops = env.carteira(0);
    env.update_deploy_allowlist(0, ops.wallet.pubkey());
    env.deploy_capital(ops.usdc, 9_000 * UNIT); // a reserva de 10% fica no caixa
    env.publish_nav(1_000_000);
    env.publish_nav(1_100_000); // marcação ACIMA do custo

    let novo = env.carteira(1_100 * UNIT);
    env.whitelist(&novo.wallet.pubkey(), true);
    let meta = env.deposit(&novo, 1_100 * UNIT);
    let e: Deposited = evento(&meta).expect("Deposited");

    assert_eq!(e.preco_usado, 1_100_000, "cunhou ao BRUTO");
    assert!(!e.pelo_piso);
    assert_eq!(e.shares, 1_000 * UNIT);
}

/// A trava é só na EMISSÃO. Quem sai no drawdown sai ao bruto — se o resgate
/// pagasse pelo piso, quem fica bancaria a diferença.
#[test]
fn resgate_no_drawdown_sai_ao_bruto() {
    let (mut env, primeiro) = cofre_em_drawdown();
    let pedido = env.solicitar_resgate(&primeiro, 1_000 * UNIT);
    let pagadora = env.caixa_da_autoridade(50_000 * UNIT);
    env.update_endereco_resgate(pagadora);
    let antes = saldo(&env.svm, &primeiro.usdc);
    env.efetivar_resgate(pedido, 900_000, primeiro.usdc);
    let pago = saldo(&env.svm, &primeiro.usdc) - antes;
    assert_eq!(
        pago,
        900 * UNIT,
        "1.000 cotas x 0,900000 — o bruto, nao o piso"
    );
}

/// O índice não muda de dono por causa do preço: a entrada do lote novo é o
/// `indice_p` de agora nos dois caminhos, e o ganho de quem já estava continua
/// sendo o dele.
#[test]
fn conservacao_do_indice_nos_dois_caminhos() {
    for (nav, pelo_piso) in [(900_000u64, true), (1_100_000u64, false)] {
        let mut env = Env::new();
        let velho = env.cotista(10_000 * UNIT);
        let ops = env.carteira(0);
        env.update_deploy_allowlist(0, ops.wallet.pubkey());
        env.deploy_capital(ops.usdc, 9_000 * UNIT);
        env.publish_nav(1_000_000);
        env.publish_nav(nav);

        /* P na gaveta ANTES do novo entrar: é dele quem já estava. */
        env.p_na_gaveta(100 * UNIT);
        env.publish_nav(nav);
        let indice_p = env.vault().indice_p;
        assert!(indice_p > 0, "o P entrou no indice");

        let novo = env.carteira(1_000 * UNIT);
        env.whitelist(&novo.wallet.pubkey(), true);
        let meta = env.deposit(&novo, 1_000 * UNIT);
        let e: Deposited = evento(&meta).expect("Deposited");
        assert_eq!(e.pelo_piso, pelo_piso);

        let p_novo = env
            .posicao_de(&novo.wallet.pubkey())
            .expect("posicao do novo");
        assert_eq!(
            p_novo.indice_entrada, indice_p,
            "quem entra depois do P nao divide o P — o preco nao muda isso"
        );
        let p_velho = env
            .posicao_de(&velho.wallet.pubkey())
            .expect("posicao do velho");
        assert_eq!(
            p_velho.indice_entrada, 0,
            "a entrada de quem ja estava nao se mexe"
        );
    }
}

/// A gênese não tem piso (supply 0 ⇒ `nav_piso` 0): o `max` devolve o NAV.
#[test]
fn genese_cunha_ao_nav() {
    let mut env = Env::new();
    /* O `Env::new()` nasce com o cofre inicializado e sem cota: o piso so' e'
    recalculado quando ha supply, entao ele fica no valor de genese. */
    let piso_na_genese = env.vault().nav_piso;
    let primeiro = env.carteira(1_000 * UNIT);
    env.whitelist(&primeiro.wallet.pubkey(), true);
    let meta = env.deposit(&primeiro, 1_000 * UNIT);
    let e: Deposited = evento(&meta).expect("Deposited");
    assert_eq!(e.nav, 1_000_000, "NAV de genese");
    assert_eq!(
        e.preco_usado,
        1_000_000.max(piso_na_genese),
        "o max com o piso de genese — e nada de surpresa no primeiro aporte"
    );
}

/// `ajustar_deployed_usdc`: só reduz, com motivo, e nunca por quem não é a
/// autoridade (a recusa de intruso está no T54).
#[test]
fn ajustar_deployed_so_reduz_e_exige_motivo() {
    let mut env = Env::new();
    let _ = env.cotista(10_000 * UNIT);
    let ops = env.carteira(0);
    env.update_deploy_allowlist(0, ops.wallet.pubkey());
    env.deploy_capital(ops.usdc, 9_000 * UNIT); // a reserva de 10% fica no caixa
    let antes = env.vault().deployed_usdc;
    assert_eq!(antes, 9_000 * UNIT);

    assert_dom_error(
        env.ajustar_deployed_usdc(antes + 1, "tentando aumentar o piso"),
        DomError::DeployedSoReduz,
        "aumentar levantaria o preco de entrada",
    );
    assert_dom_error(
        env.ajustar_deployed_usdc(antes, "mesmo valor nao e' correcao"),
        DomError::DeployedSoReduz,
        "igual tambem nao passa",
    );
    assert_dom_error(
        env.ajustar_deployed_usdc(antes - 1, "curto"),
        DomError::MotivoInvalido,
        "motivo de menos de 8 caracteres",
    );

    assert_ok(
        env.ajustar_deployed_usdc(
            antes - 1_000 * UNIT,
            "erro de contabilidade conferido no laudo",
        ),
        "reduzir com motivo",
    );
    assert_eq!(env.vault().deployed_usdc, 8_000 * UNIT);
}
