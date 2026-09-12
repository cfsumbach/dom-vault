//! Grupo B da matriz — **cap de 25% e grandfathering** (T08–T18).
//!
//! O cap é checado contra o supply, não contra um teto fixo por carteira: vale
//! no transfer (hook) e no mint (`deposit`), com a mesma conta —
//! `saldo_pos * 100 > supply_pos * 25` rejeita, estritamente maior (D3).
//!
//! **Mas é admissão, não invariante.** Ele barra quem *entra* acima de 25%; não
//! impede que alguém *fique* acima disso quando o supply encolhe por queima de
//! terceiro. T67 é o contraexemplo, e o texto anterior deste cabeçalho — "o cap
//! é invariante de supply" — dizia mais do que o código entrega (F4-02).
//!
//! Cenário base de quase todos: dois cotistas, 800 e 200 cotas, supply 1.000.
//! Números redondos para que 25% caia em valor exato e a borda seja testável.
//!
//! **T14 não está aqui.** Depende de `accrue_performance` mintar `fee_share`,
//! que é do bloco de apuração — ver o relatório do bloco.

#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{error::DomError, events::CapEnforcementEnabled},
    solana_keypair::Keypair,
    solana_signer::Signer,
};

/// 800 + 200 = 1.000 cotas. Cap desligado ainda.
fn cenario() -> (Env, Holder, Holder) {
    let mut env = Env::new();
    let grande = env.cotista(800 * UNIT);
    let pequeno = env.cotista(200 * UNIT);
    assert_eq!(env.supply_dom(), 1_000 * UNIT);
    (env, grande, pequeno)
}

// ---------------------------------------------------------------------------
// T08 / T09 — a borda dos 25%
// ---------------------------------------------------------------------------

/// Transferência que deixaria o destino acima de 25% do supply rejeita.
#[test]
fn t08_transfer_acima_do_cap_rejeita() {
    let (mut env, grande, pequeno) = cenario();
    env.enable_cap();

    // 200 + 100 = 300 de 1.000 = 30%.
    assert_dom_error(
        env.transfer(&grande, &pequeno.dom, &pequeno.wallet.pubkey(), 100 * UNIT),
        DomError::CapExceeded,
        "T08 destino a 30%",
    );
    assert_eq!(
        env.saldo_dom(&pequeno),
        200 * UNIT,
        "T08: saldo do destino intacto apos a rejeicao"
    );
}

/// **Exatamente** 25,000000% passa: quem rejeita é o estritamente maior (D3).
/// Uma unidade a mais já não passa — as duas metades no mesmo teste, porque o
/// valor desta borda é ela ser exata.
#[test]
fn t09_exatamente_25_por_cento_passa() {
    let (mut env, grande, pequeno) = cenario();
    env.enable_cap();

    // 200 + 50 = 250 de 1.000 = 25,000000% exato.
    assert_ok(
        env.transfer(&grande, &pequeno.dom, &pequeno.wallet.pubkey(), 50 * UNIT),
        "T09 destino a 25% exato",
    );
    assert_eq!(env.saldo_dom(&pequeno), 250 * UNIT);

    // Mais uma unidade mínima: 250,000001 de 1.000 — rejeita.
    assert_dom_error(
        env.transfer(&grande, &pequeno.dom, &pequeno.wallet.pubkey(), 1),
        DomError::CapExceeded,
        "T09 uma unidade acima do exato",
    );
}

// ---------------------------------------------------------------------------
// T10 — cap inativo na gênese
// ---------------------------------------------------------------------------

/// Sem `cap_enforced`, dois cotistas a 50% cada e transferência que leva o
/// destino a 60%: passa. O cap nasce desligado (D3) e o fundo começa
/// concentrado por definição.
#[test]
fn t10_cap_inativo_permite_concentracao() {
    let mut env = Env::new();
    let a = env.cotista(500 * UNIT);
    let b = env.cotista(500 * UNIT);
    assert!(!env.vault().cap_enforced, "cap comeca desligado");

    assert_ok(
        env.transfer(&a, &b.dom, &b.wallet.pubkey(), 100 * UNIT),
        "T10 destino a 60% com cap inativo",
    );
    assert_eq!(env.saldo_dom(&b), 600 * UNIT);
}

// ---------------------------------------------------------------------------
// T11 — ativação one-way
// ---------------------------------------------------------------------------

/// Primeira ativação emite evento; a segunda chamada falha. Não existe caminho
/// de volta — é o que dá sentido ao grandfathering (D3).
#[test]
fn t11_ativacao_do_cap_e_one_way() {
    let (mut env, _grande, _pequeno) = cenario();

    let meta = env.enable_cap();
    assert!(
        tem_evento::<CapEnforcementEnabled>(&meta),
        "T11: evento de ativacao emitido"
    );
    assert!(env.vault().cap_enforced);

    let authority = env.authority.insecure_clone();
    assert_dom_error(
        env.enable_cap_raw(&authority),
        DomError::CapAlreadyEnforced,
        "T11 segunda ativacao",
    );
    assert!(
        env.vault().cap_enforced,
        "T11: estado continua ativo apos a segunda tentativa"
    );
}

/// Ativar o cap não é para qualquer um (T54, na parte que toca o cap).
#[test]
fn t11b_ativacao_por_nao_autoridade_rejeita() {
    let (mut env, _grande, _pequeno) = cenario();

    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), SOL).unwrap();

    assert_dom_error(
        env.enable_cap_raw(&intruso),
        DomError::Unauthorized,
        "T11b ativacao por intruso",
    );
    assert!(!env.vault().cap_enforced);
}

// ---------------------------------------------------------------------------
// T12 / T13 — grandfathering
// ---------------------------------------------------------------------------

/// Quem estava acima de 25% na ativação **mantém** a posição: nenhuma operação
/// reduz saldo alheio, e o cotista continua podendo transferir para fora.
#[test]
fn t12_grandfathered_mantem_posicao_e_pode_enviar() {
    let (mut env, grande, pequeno) = cenario();
    env.enable_cap();

    assert_eq!(
        env.saldo_dom(&grande),
        800 * UNIT,
        "T12: ativacao nao mexe em saldo"
    );

    // Continua podendo mandar para fora — 200 + 50 = 250 = 25% exato no destino.
    assert_ok(
        env.transfer(&grande, &pequeno.dom, &pequeno.wallet.pubkey(), 50 * UNIT),
        "T12 grandfathered enviando",
    );
    assert_eq!(env.saldo_dom(&grande), 750 * UNIT, "ainda acima de 25%");
}

/// O que o grandfathered não pode é **aumentar**. Nenhum bookkeeping separado:
/// a própria checagem do cap barra, porque o saldo dele já está acima (D3).
#[test]
fn t13_grandfathered_nao_pode_receber() {
    let (mut env, grande, pequeno) = cenario();
    env.enable_cap();

    assert_dom_error(
        env.transfer(&pequeno, &grande.dom, &grande.wallet.pubkey(), 10 * UNIT),
        DomError::CapExceeded,
        "T13 grandfathered recebendo",
    );
    assert_eq!(env.saldo_dom(&grande), 800 * UNIT);
}

// ---------------------------------------------------------------------------
// T15 / T17 — cap no caminho de mint
// ---------------------------------------------------------------------------

/// Depósito comum de quem já está acima de 25% rejeita — inclusive de sócio
/// (T15). O cap não olha quem é a carteira, olha o saldo resultante.
///
/// A outra metade do T15 — `fee_share` isento — depende de
/// `accrue_performance` e fica para o bloco de apuração.
#[test]
fn t15_deposito_de_quem_esta_acima_do_cap_rejeita() {
    let (mut env, grande, _pequeno) = cenario();
    env.enable_cap();

    // A carteira do "sócio" já tem 80%; qualquer aporte comum piora.
    let socio = Holder {
        wallet: grande.wallet.insecure_clone(),
        dom: grande.dom,
        usdc: grande.usdc,
    };
    let usdc_extra = 200 * UNIT;
    mint_tokens(
        &mut env.svm,
        &env.payer,
        &token_classic(),
        &env.usdc_mint.clone(),
        &env.usdc_authority.insecure_clone(),
        &socio.usdc,
        usdc_extra,
    );

    assert_dom_error(
        env.deposit_raw(&socio, usdc_extra),
        DomError::CapExceeded,
        "T15 deposito comum de socio acima do cap",
    );
    assert_eq!(
        env.saldo_dom(&grande),
        800 * UNIT,
        "T15: mint revertido junto com a transacao"
    );
    assert_eq!(
        env.saldo_usdc(&socio),
        usdc_extra,
        "T15: USDC devolvido — a transacao e atomica"
    );
}

/// Depósito que deixaria o **novo** cotista acima de 25% rejeita; logo abaixo da
/// borda passa. O contra-teste é o que prova que a rejeição foi o cap, e não o
/// depósito estar quebrado.
#[test]
fn t17_deposito_acima_do_cap_rejeita() {
    let (mut env, _grande, _pequeno) = cenario();
    env.enable_cap();

    // 400 de (1.000 + 400) = 28,57% — rejeita.
    let novo = env.carteira(400 * UNIT);
    assert_dom_error(
        env.deposit_raw(&novo, 400 * UNIT),
        DomError::CapExceeded,
        "T17 deposito a 28,5%",
    );
    assert_eq!(env.saldo_dom(&novo), 0);
    assert_eq!(env.supply_dom(), 1_000 * UNIT, "supply intacto");

    // 333,333333 de (1.000 + 333,333333) = 24,999...% — passa.
    assert_ok(
        env.deposit_raw(&novo, 333_333_333),
        "T17 contra-teste logo abaixo da borda",
    );
    assert_eq!(env.saldo_dom(&novo), 333_333_333);
}

// ---------------------------------------------------------------------------
// T16 — supply cresce e o grandfathered volta a caber
// ---------------------------------------------------------------------------

/// Grandfathering não é marca permanente: é consequência da conta. Quando o
/// supply cresce a ponto de a posição cair abaixo de 25%, a carteira volta a
/// poder receber — sem nenhuma instrução de "perdão".
///
/// O supply cresce por depósitos de terceiros, cada um respeitando o próprio
/// cap: um aporte de um terço do supply corrente deixa o novo cotista em 25%
/// exatos.
#[test]
fn t16_supply_crescendo_liberta_o_grandfathered() {
    let (mut env, grande, pequeno) = cenario();
    env.enable_cap();

    let mut rodadas = 0;
    while (env.saldo_dom(&grande) as u128) * 100 > (env.supply_dom() as u128) * 25 {
        assert!(rodadas < 8, "T16: supply nao cresceu como esperado");
        let aporte = env.supply_dom() / 3;
        let novo = env.carteira(aporte);
        env.deposit(&novo, aporte);
        rodadas += 1;
    }

    // Agora o antigo dominante cabe no cap e volta a poder receber.
    assert_ok(
        env.transfer(&pequeno, &grande.dom, &grande.wallet.pubkey(), 10 * UNIT),
        "T16 grandfathered recebendo depois do crescimento",
    );
    assert_eq!(env.saldo_dom(&grande), 810 * UNIT);
}

// ---------------------------------------------------------------------------
// T18 — queima empurra remanescente acima do cap, e nada reverte
// ---------------------------------------------------------------------------

/// **Cobertura por proxy.** O caso da matriz é a queima de cota empurrando um
/// remanescente acima de 25% — hoje quem queima é o `efetivar_resgate_capital`.
/// Aqui a queima é feita direto no Token-2022 pelo próprio dono: o efeito sobre o
/// cap é idêntico, porque o que importa é o supply cair, e o proxy não depende de
/// abrir um resgate de 180 dias só para provar aritmética de cap.
///
/// O ponto do teste: **nada reverte**. Ninguém aumentou saldo, então nenhuma
/// checagem dispara. Quem subiu passivamente vira grandfathered e para de poder
/// receber — que é o comportamento aceito na D3.
#[test]
fn t18_queima_empurra_remanescente_sem_reverter() {
    let (mut env, grande, pequeno) = cenario();
    env.enable_cap();

    // Supply 1.000 -> 400. Os dois passam a 50% sem terem recebido nada.
    assert_ok(env.burn(&grande, 600 * UNIT), "T18 queima");

    assert_eq!(env.supply_dom(), 400 * UNIT);
    assert_eq!(env.saldo_dom(&grande), 200 * UNIT);
    assert_eq!(env.saldo_dom(&pequeno), 200 * UNIT, "T18: saldo intocado");

    // A partir daí, aumentar está bloqueado para os dois.
    assert_dom_error(
        env.transfer(&grande, &pequeno.dom, &pequeno.wallet.pubkey(), UNIT),
        DomError::CapExceeded,
        "T18 remanescente tentando aumentar",
    );
}

// ---------------------------------------------------------------------------
// T67 — o cap é admissão, não invariante (auditoria F4-02, ordem 3)
// ---------------------------------------------------------------------------

/// T67 — **queima de terceiro empurra um cotista acima de 25%, e nada rejeita.**
///
/// O cap é conferido em dois pontos: no mint (`deposit`) e no destino da
/// transferência (hook). Nenhum dos dois roda quando **outro** cotista resgata —
/// e o resgate queima cota, encolhendo o supply. O percentual de quem ficou sobe
/// sozinho.
///
/// Quatro cotistas com 25% exatos cada. Um resgata 600 cotas. Supply cai de
/// 4.000 para 3.400, e os três que não fizeram nada passam a ter 29,4%.
///
/// Achado da invariante negativa 3 do T66, isolado aqui em cenário
/// determinístico. Não é bug de memória nem de contabilidade: é a diferença
/// entre o que a **N9** promete ("máx 25% do supply") e o que o programa
/// entrega (nenhuma admissão acima de 25%).
#[test]
fn t67_queima_de_terceiro_empurra_cotista_acima_de_25_por_cento() {
    let mut env = Env::new();
    // `a` guarda 200 USDC no bolso para o teste de admissao do fim.
    let a = env.carteira(1_200 * UNIT);
    env.deposit(&a, 1_000 * UNIT);
    let _b = env.cotista(1_000 * UNIT);
    let _c = env.cotista(1_000 * UNIT);
    let d = env.cotista(1_000 * UNIT);
    assert_eq!(env.supply_dom(), 4_000 * UNIT);

    env.enable_cap();

    // Ponto de partida: 25% exatos, que é o limite inclusivo (T09).
    assert_eq!(
        env.saldo_dom(&a) as u128 * 100,
        env.supply_dom() as u128 * 25
    );

    // `d` queima 600 cotas — o mesmo efeito de supply que a efetivação de um
    // resgate teria, sem o teatro de abrir e liquidar um pedido.
    assert_ok(env.burn(&d, 600 * UNIT), "queima direta");

    let supply = env.supply_dom();
    let saldo_a = env.saldo_dom(&a);
    assert_eq!(supply, 3_400 * UNIT, "600 cotas queimadas");
    assert_eq!(saldo_a, 1_000 * UNIT, "`a` nao encostou nas cotas dele");

    // 1.000 / 3.400 = 29,41%. Acima do cap, com o cap LIGADO, sem nenhuma
    // instrucao ter sido rejeitada.
    assert!(
        saldo_a as u128 * 100 > supply as u128 * 25,
        "esperado passar de 25%: {saldo_a} de {supply}"
    );
    assert!(env.vault().cap_enforced, "o cap estava ligado o tempo todo");

    // O que o cap ainda faz: barra a ENTRADA. `a` nao consegue aumentar.
    assert_dom_error(
        env.deposit_raw(&a, 200 * UNIT),
        DomError::CapExceeded,
        "T67 deposito novo de quem ja esta acima do cap",
    );
}
