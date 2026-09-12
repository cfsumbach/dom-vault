//! Bloco 3 da F2 — **a porta operacional** (T81–T88).
//!
//! A F1 provou um cofre fechado: dinheiro entrava por `deposit` e só saía por
//! resgate ou taxa, para destinos **derivados** — o dono do pedido na fila, o
//! sócio do livro. `deploy_capital` é a primeira saída para endereço de
//! **lista**, com valor livre.
//!
//! Cada trava tem contra-teste na mesma execução: o teste que prova a recusa
//! prova, logo abaixo, que o caminho legítimo passa. Recusa sem contra-teste não
//! distingue "trava funcionando" de "instrução quebrada".

#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{
        constants::{MIN_RESGATE_CAPITAL_USDC, NAV_SCALE},
        error::DomError,
        events::{CapitalDeployed, CapitalReturned},
    },
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const APORTE: u64 = 10_000 * UNIT;

/// Cofre com caixa e uma carteira operacional já registrada na vaga 0.
fn cofre_com_ops() -> (Env, Holder, Holder) {
    let mut env = Env::new();
    let cotista = env.cotista(APORTE);
    let ops = env.carteira(0);
    env.update_deploy_allowlist(0, ops.wallet.pubkey());
    (env, cotista, ops)
}

// ---------------------------------------------------------------------------
// T81 — trava 1: autoridade
// ---------------------------------------------------------------------------

/// T81 — `deploy_capital` por quem não é a autoridade é recusado.
///
/// A `authority` é o Vault PDA do Squads (D2): uma assinatura não move nada, o
/// caminho é proposta com quorum. Mesma prova do F1, agora sobre a instrução que
/// tira dinheiro do cofre.
#[test]
fn t81_deploy_por_nao_autoridade_rejeita() {
    let (mut env, _cotista, ops) = cofre_com_ops();
    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), SOL).unwrap();

    assert_dom_error(
        env.deploy_capital_raw(&intruso, ops.usdc, 1_000 * UNIT),
        DomError::Unauthorized,
        "T81 intruso mandando capital para fora",
    );
    assert_eq!(env.saldo_usdc(&ops), 0, "nada saiu");

    // Contra-teste: a autoridade passa.
    env.deploy_capital(ops.usdc, 1_000 * UNIT);
    assert_eq!(env.saldo_usdc(&ops), 1_000 * UNIT);
}

/// T81b — `update_deploy_allowlist` também é privilegiada. Se não fosse, a
/// trava 2 seria decorativa: bastaria registrar o próprio endereço.
#[test]
fn t81b_allowlist_por_nao_autoridade_rejeita() {
    let mut env = Env::new();
    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), SOL).unwrap();

    assert_dom_error(
        env.update_deploy_allowlist_raw(&intruso, 0, intruso.pubkey()),
        DomError::Unauthorized,
        "T81b intruso registrando a propria carteira",
    );
    assert_eq!(
        env.vault().deploy_allowlist[0],
        anchor_lang::prelude::Pubkey::default(),
        "a vaga continua livre"
    );
}

// ---------------------------------------------------------------------------
// T82 — trava 2: allowlist
// ---------------------------------------------------------------------------

/// T82 — destino fora da allowlist é recusado, mesmo com a autoridade
/// assinando.
#[test]
fn t82_destino_fora_da_allowlist_rejeita() {
    let (mut env, _cotista, ops) = cofre_com_ops();
    let estranho = env.carteira(0);

    assert_dom_error(
        env.deploy_capital_raw(&env.authority.insecure_clone(), estranho.usdc, 1_000 * UNIT),
        DomError::DestinationNotAllowed,
        "T82 destino nao registrado",
    );
    assert_eq!(env.saldo_usdc(&estranho), 0);

    // Contra-teste: o destino registrado recebe.
    env.deploy_capital(ops.usdc, 1_000 * UNIT);
    assert_eq!(env.saldo_usdc(&ops), 1_000 * UNIT);
}

/// T82b — a allowlist guarda a **carteira**, não a ATA. Registrar o dono faz
/// qualquer conta de USDC dele servir; a conta é conferida contra o dono e
/// contra o mint do cofre.
#[test]
fn t82b_allowlist_casa_pelo_dono_da_conta() {
    let (mut env, _cotista, ops) = cofre_com_ops();

    // Segunda conta de USDC da MESMA carteira operacional.
    let outra = env.conta_usdc_extra(&ops.wallet.pubkey());
    env.deploy_capital(outra, 500 * UNIT);
    assert_eq!(
        env.saldo_usdc_da_conta(outra),
        500 * UNIT,
        "T82b conta nova do mesmo dono serve"
    );
}

/// T82c — **carteira de sócio nunca entra na allowlist** (D-F2-01).
///
/// Dinheiro de investidor circula pelos destinos operacionais; dinheiro de sócio
/// sai só por `redeem_fee_share`, contra o livro. Sem esta trava, uma proposta
/// aprovada distraidamente transformaria `deploy_capital` num caminho de
/// pagamento a sócio que não passa pelo HWM nem pelo livro.
#[test]
fn t82c_socio_nao_pode_ser_destino() {
    let mut env = Env::new();
    let socio = env.socios[0].wallet.pubkey();

    assert_dom_error(
        env.update_deploy_allowlist_raw(&env.authority.insecure_clone(), 0, socio),
        DomError::SocioNaoPodeSerDestino,
        "T82c socio como destino operacional",
    );

    // Contra-teste: carteira que não é de sócio entra.
    let ops = env.carteira(0);
    env.update_deploy_allowlist(0, ops.wallet.pubkey());
    assert_eq!(env.vault().deploy_allowlist[0], ops.wallet.pubkey());
}

/// T82d — limpar a vaga tranca o destino de novo. `Pubkey::default()` nunca casa
/// com conta de token, então vaga limpa é vaga fechada.
#[test]
fn t82d_limpar_a_vaga_fecha_o_destino() {
    let (mut env, _cotista, ops) = cofre_com_ops();
    env.deploy_capital(ops.usdc, 100 * UNIT);

    env.update_deploy_allowlist(0, anchor_lang::prelude::Pubkey::default());
    assert_dom_error(
        env.deploy_capital_raw(&env.authority.insecure_clone(), ops.usdc, 100 * UNIT),
        DomError::DestinationNotAllowed,
        "T82d destino removido da lista",
    );
    assert_eq!(env.saldo_usdc(&ops), 100 * UNIT, "so o primeiro passou");
}

// ---------------------------------------------------------------------------
// T83 / T84 — trava 3: reserva e fila
// ---------------------------------------------------------------------------

/// T83 — a saída não pode furar a **reserva de 10%**.
///
/// Patrimônio 10.000 → reserva 1.000. O caixa tem 10.000, então cabe sair 9.000
/// e nem um micro a mais.
#[test]
fn t83_reserva_e_piso_do_deploy() {
    let (mut env, _cotista, ops) = cofre_com_ops();
    assert_eq!(env.caixa(), APORTE);

    assert_dom_error(
        env.deploy_capital_raw(&env.authority.insecure_clone(), ops.usdc, 9_000 * UNIT + 1),
        DomError::ReserveViolation,
        "T83 um micro alem da reserva",
    );

    // Contra-teste: o limite exato passa.
    env.deploy_capital(ops.usdc, 9_000 * UNIT);
    assert_eq!(env.caixa(), 1_000 * UNIT, "sobrou exatamente a reserva");
    assert_eq!(env.vault().deployed_usdc, 9_000 * UNIT, "capital em campo");
}

/// T84 — **o pedido de resgate NÃO reduz mais o que pode sair, e é de propósito.**
///
/// O piso era reserva **mais** o passivo congelado da fila, porque a fila D+30 se
/// pagava do treasury. O resgate de capital se paga do `endereco_resgate`, que a
/// mesa abastece de **fora** do cofre — reservar caixa aqui congelaria dinheiro
/// contra obrigação que não sai daqui.
///
/// Quem cobre esse risco é a trava de `resgates_em_atraso`, e ela é mais forte:
/// não reserva caixa, **para o capital de sair** (provado em `resgate_capital.rs`).
#[test]
fn t84_pedido_aberto_nao_reduz_o_que_pode_sair() {
    let (mut env, cotista, ops) = cofre_com_ops();

    env.solicitar_resgate(&cotista, 5_000 * UNIT);

    // O pedido move as cotas para o escrow, mas **não queima**: o supply segue
    // 10.000 até a efetivação. Patrimônio intacto, reserva de 10% = 1.000.
    let reserva = 1_000 * UNIT;
    assert_dom_error(
        env.deploy_capital_raw(
            &env.authority.insecure_clone(),
            ops.usdc,
            APORTE - reserva + 1,
        ),
        DomError::ReserveViolation,
        "a reserva continua sendo piso",
    );

    // E o limite exato passa: o pedido aberto não tira nada além dela.
    env.deploy_capital(ops.usdc, APORTE - reserva);
    assert_eq!(env.caixa(), reserva);
}

/// T84b — **o resgate é honrado com o capital todo em campo.**
///
/// É o teste que liga as duas pontas do desenho da mesa: o dinheiro do resgate
/// não vem do cofre, então esvaziar o treasury até a reserva não impede pagar.
/// Era exatamente o caso que travava a fila D+30 e que motivou o mecanismo novo.
#[test]
fn t84b_resgate_e_honrado_com_o_caixa_no_piso() {
    let (mut env, cotista, ops) = cofre_com_ops();
    let pagadora = env.caixa_da_autoridade(20_000 * UNIT);
    env.update_endereco_resgate(pagadora);

    let id = env.solicitar_resgate(&cotista, 5_000 * UNIT);
    env.deploy_capital(ops.usdc, 9_000 * UNIT);
    assert_eq!(env.caixa(), 1_000 * UNIT, "caixa no piso da reserva");

    let antes = env.saldo_usdc(&cotista);
    let nav = env.vault().nav;
    env.efetivar_resgate(id, nav, cotista.usdc);

    assert_eq!(
        env.saldo_usdc(&cotista) - antes,
        5_000 * UNIT * nav / dom_vault::constants::NAV_SCALE,
        "pago mesmo com o cofre no piso — a fonte esta fora"
    );
    assert_eq!(env.caixa(), 1_000 * UNIT, "o treasury nao foi tocado");
}

// ---------------------------------------------------------------------------
// T85 — pausa
// ---------------------------------------------------------------------------

/// T85 — cofre pausado não deixa capital sair.
///
/// `pause` é o freio de crise, e mandar dinheiro para o trabalho no meio de uma é
/// o oposto do que ele existe para fazer. Não está no mandato; segue o racional
/// do R1, igual ao que já vale para apuração e `fee_share`.
#[test]
fn t85_pause_bloqueia_deploy() {
    let (mut env, _cotista, ops) = cofre_com_ops();
    env.pause();

    assert_dom_error(
        env.deploy_capital_raw(&env.authority.insecure_clone(), ops.usdc, 100 * UNIT),
        DomError::Paused,
        "T85 deploy com cofre pausado",
    );

    env.unpause();
    env.deploy_capital(ops.usdc, 100 * UNIT);
    assert_eq!(env.saldo_usdc(&ops), 100 * UNIT, "T85 contra-teste");
}

// ---------------------------------------------------------------------------
// T86 / T87 — a volta
// ---------------------------------------------------------------------------

/// T86 — `return_capital` reconcilia o caixa: saldo e evento.
#[test]
fn t86_return_capital_reconcilia_o_caixa() {
    let (mut env, _cotista, ops) = cofre_com_ops();
    env.deploy_capital(ops.usdc, 5_000 * UNIT);
    assert_eq!(env.caixa(), 5_000 * UNIT);
    assert_eq!(env.vault().deployed_usdc, 5_000 * UNIT);

    let meta = env.return_capital(&ops, 2_000 * UNIT);
    assert!(tem_evento::<CapitalReturned>(&meta), "evento de volta");

    assert_eq!(env.caixa(), 7_000 * UNIT, "o caixa recebeu de volta");
    assert_eq!(env.saldo_usdc(&ops), 3_000 * UNIT, "sobrou em campo");
    assert_eq!(
        env.vault().deployed_usdc,
        3_000 * UNIT,
        "o contador de capital em campo abateu"
    );
}

/// T87 — **entrar é irrestrito.** A allowlist controla por onde o dinheiro sai;
/// exigir proposta para devolver só criaria um caminho em que o dinheiro fica
/// preso fora esperando quorum.
///
/// E devolver **mais do que saiu** é o caso bom: significa lucro. O contador de
/// capital em campo mede custo, então tem piso em zero — o lucro aparece no NAV,
/// que é onde ele tem que aparecer.
#[test]
fn t87_qualquer_um_devolve_e_o_lucro_nao_estoura_o_contador() {
    let (mut env, _cotista, ops) = cofre_com_ops();
    env.deploy_capital(ops.usdc, 1_000 * UNIT);

    // Um terceiro qualquer, fora da allowlist, devolve ao cofre.
    let estranho = env.carteira(3_000 * UNIT);
    env.return_capital(&estranho, 3_000 * UNIT);

    assert_eq!(env.caixa(), 9_000 * UNIT + 3_000 * UNIT);
    assert_eq!(
        env.vault().deployed_usdc,
        0,
        "devolveu mais do que saiu: o contador tem piso em zero, nao estoura"
    );
}

// ---------------------------------------------------------------------------
// T88 — o evento e o índice
// ---------------------------------------------------------------------------

/// T88 — o `CapitalDeployed` carrega o que o backend de NAV precisa, e o índice
/// da allowlist é conferido.
#[test]
fn t88_evento_e_indice() {
    let (mut env, _cotista, ops) = cofre_com_ops();

    let meta = env.deploy_capital(ops.usdc, 1_234 * UNIT);
    assert!(tem_evento::<CapitalDeployed>(&meta), "evento de saida");

    // Índice fora da lista é recusado com nome, não com panico de slice.
    assert_dom_error(
        env.update_deploy_allowlist_raw(&env.authority.insecure_clone(), 2, ops.wallet.pubkey()),
        DomError::AllowlistIndexOutOfRange,
        "T88 indice 2 numa lista de 2 vagas",
    );

    // Valor zero não é movimento.
    assert_dom_error(
        env.deploy_capital_raw(&env.authority.insecure_clone(), ops.usdc, 0),
        DomError::ZeroDeploy,
        "T88 deploy de zero",
    );
    assert_eq!(env.vault().nav, NAV_SCALE, "nada disso mexeu no NAV");
    let _ = MIN_RESGATE_CAPITAL_USDC;
}
