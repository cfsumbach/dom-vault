//! **Os testes obrigatórios da D-F2-43 §6, (1)–(4)**, mais a medição de CU de
//! cada instrução que o Upgrade J tocou.
//!
//! (1) janela após fechamento com resgate parcial no meio, e aporte com janela
//!     aberta → recusa;
//! (2) `publish_nav` com gaveta diminuída → recusa (também em `tests/gaveta.rs`,
//!     com o destrave ao devolver);
//! (3) fechamento com `p_ciclo ≠ gaveta` → recusa — nas duas formas em que o
//!     desvio pode existir (P da proposta ≠ gaveta; gaveta diminuída);
//! (4) dois fechamentos seguidos sem P → ganho zero, sem pânico numérico.
//!
//! CU: `cargo test --test obrigatorios_j -- --nocapture` imprime a tabela.
#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{
        constants::{DISTRIBUICAO_INTERVAL, MIN_NAV_PUBLISH_INTERVAL},
        error::DomError,
        indice::ganho,
        state::AfiliadoDoFechamento,
    },
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const E6: u64 = 1_000_000;

// ---------------------------------------------------------------------------
// (1) janela com resgate parcial no meio; aporte na janela recusado
// ---------------------------------------------------------------------------

/// A: 10.000 desde o começo. P1 = 600. B: 10.000 entra depois (índice 0,06).
/// P2 = 800. Fechamento. A tem acerto pendente de DÉBITO (entrou antes, ganhou
/// mais que a diluição cobrou); B, de CRÉDITO. A pede resgate de metade com a
/// janela aberta — o `acertar` vai na frente, a transferência para o escrow
/// passa — e depois saca: o direito é sobre as cotas que SOBRARAM.
#[test]
fn janela_com_resgate_parcial_no_meio_e_aporte_recusado() {
    let mut env = Env::new();
    let a = env.cotista(10_000 * E6);
    env.p_na_gaveta(600 * E6);
    env.publish_nav_pela_mesa(1_060_000); // (10.000 + 600) ÷ 10.000
    let b = env.cotista(10_000 * E6);
    env.p_na_gaveta(800 * E6);
    let supply = env.supply_dom();
    let piso =
        ((20_000u128 * E6 as u128 + 1_400 * E6 as u128) * E6 as u128 / supply as u128) as u64;
    env.publish_nav_pela_mesa(piso);
    let v = env.vault();
    let ganho_a = ganho(
        env.saldo_dom(&a),
        env.posicao_de(&a.wallet.pubkey()).unwrap().indice_entrada,
        v.indice_ciclo,
        v.indice_p,
    )
    .unwrap();
    let ganho_b = ganho(
        env.saldo_dom(&b),
        env.posicao_de(&b.wallet.pubkey()).unwrap().indice_entrada,
        v.indice_ciclo,
        v.indice_p,
    )
    .unwrap();
    assert!((ganho_a + ganho_b).abs_diff(1_400 * E6) < 4, "Σ ganho = P");
    assert!(
        ganho_a > 1_000 * E6 && ganho_b < 400 * E6,
        "A ganhou o P1 inteiro e metade do P2; B so' metade do P2"
    );

    env.deposit_especial(1_400 * E6);
    let v = env.vault();
    assert!(v.lucro_sacavel_restante > 0, "janela aberta");
    let taxa = v.perf_fee_bps_total as u128;

    // aporte com a janela aberta: recusa, nas duas portas
    let novato = env.carteira(1_000 * E6);
    assert_dom_error(
        env.deposit_raw(&novato, 1_000 * E6),
        DomError::JanelaDeLucroAberta,
        "deposit na janela",
    );
    assert_dom_error(
        env.deposit_para_raw(&novato, &b, 500 * E6),
        DomError::JanelaDeLucroAberta,
        "deposit_para na janela",
    );

    // A: débito pendente → sem `acertar` assinado a transferência (e o resgate) não passam
    let cotas_a_antes = env.saldo_dom(&a);
    assert!(
        env.transfer(&a, &b.dom, &b.wallet.pubkey(), 100 * E6)
            .is_err(),
        "janela aberta E acerto pendente: a transferencia comum nao passa"
    );
    // o resgate atravessa a janela: acertar (A assina) + escrow
    let id = env.solicitar_resgate(&a, 5_000 * E6);
    assert_eq!(env.pedido(id).cotas, 5_000 * E6);
    let cotas_a = env.saldo_dom(&a);
    let debito_a = (ganho_a as u128 * taxa / 10_000) as u64; // devido
    assert!(
        cotas_a < cotas_a_antes - 5_000 * E6,
        "o debito de A saiu no acerto: {cotas_a} < {}",
        cotas_a_antes - 5_000 * E6
    );
    let pos_a = env.posicao_de(&a.wallet.pubkey()).unwrap();
    assert_eq!(pos_a.indice_diluicao_visto, v.indice_diluicao, "A acertada");

    // A saca: o direito e' sobre as cotas que sobraram (a metade que ficou, menos o debito)
    let esperado = (ganho(
        cotas_a,
        pos_a.indice_entrada,
        v.indice_ciclo_anterior,
        v.indice_ciclo,
    )
    .unwrap() as u128
        * (10_000 - taxa)
        / 10_000) as u64;
    let usdc_antes = env.saldo_usdc(&a);
    env.sacar_lucro(&a);
    let sacou = env.saldo_usdc(&a) - usdc_antes;
    assert_eq!(
        sacou,
        esperado.min(v.lucro_sacavel_restante),
        "direito proporcional as cotas restantes"
    );
    assert!(
        sacou < (ganho_a as u128 * (10_000 - taxa) / 10_000) as u64 / 2 + E6,
        "menos da metade do que teria sem o resgate (cotas a menos)"
    );
    let _ = debito_a;

    // B: crédito pendente — qualquer um acerta, sem assinatura; depois saca o dele
    let cotas_b_antes = env.saldo_dom(&b);
    assert_ok(env.acertar_raw(&b, false), "credito sem assinatura");
    assert!(env.saldo_dom(&b) > cotas_b_antes, "B recebeu o credito");
    let usdc_b = env.saldo_usdc(&b);
    // o direito de B: sobre as cotas que ele tem na hora do saque (ja' com o credito)
    let esperado_b = (ganho(
        env.saldo_dom(&b),
        env.posicao_de(&b.wallet.pubkey()).unwrap().indice_entrada,
        v.indice_ciclo_anterior,
        v.indice_ciclo,
    )
    .unwrap() as u128
        * (10_000 - taxa)
        / 10_000) as u64;
    env.sacar_lucro(&b);
    let sacou_b = env.saldo_usdc(&b) - usdc_b;
    assert!(sacou_b.abs_diff(esperado_b) <= 2, "B sacou {sacou_b}, esperado {esperado_b}, restante antes {} cotas_b {} entrada {} ic_ant {} ic {}", v.lucro_sacavel_restante, env.saldo_dom(&b), env.posicao_de(&b.wallet.pubkey()).unwrap().indice_entrada, v.indice_ciclo_anterior, v.indice_ciclo);
}

// ---------------------------------------------------------------------------
// (2) publish_nav com gaveta diminuída → recusa (pelo oráculo)
// ---------------------------------------------------------------------------

#[test]
fn publish_nav_pelo_oraculo_com_gaveta_diminuida_e_recusado() {
    let mut env = Env::new();
    let vault1 = Keypair::new();
    env.svm.airdrop(&vault1.pubkey(), SOL).unwrap();
    let gaveta = env.criar_gaveta_de(&vault1.pubkey());
    env.gaveta = gaveta;
    let _c = env.cotista(5_000 * E6);
    env.p_na_gaveta(500 * E6);
    env.publish_nav_pela_mesa(1_100_000);
    assert_eq!(env.vault().gaveta_saldo_visto, 500 * E6);

    // a dona tira 1 micro
    let fora = env.carteira(0);
    let usdc_mint = env.usdc_mint;
    let saida = spl_token_2022_interface::instruction::transfer_checked(
        &token_classic(),
        &gaveta,
        &usdc_mint,
        &fora.usdc,
        &vault1.pubkey(),
        &[],
        1,
        6,
    )
    .unwrap();
    let payer = env.payer.insecure_clone();
    assert_ok(
        send(&mut env.svm, &payer, &[saida], &[&vault1]),
        "saida de 1 micro",
    );

    env.avancar_sem_oraculo(MIN_NAV_PUBLISH_INTERVAL);
    assert_dom_error(
        env.publish_nav_pelo_oraculo_raw(1_100_000),
        DomError::GavetaDiminuiu,
        "oraculo com a gaveta 1 micro menor",
    );
    assert_dom_error(
        env.sincronizar_gaveta_raw(),
        DomError::GavetaDiminuiu,
        "sincronizar com a gaveta menor",
    );
    env.p_na_gaveta(1);
    assert_ok(
        env.publish_nav_pelo_oraculo_raw(1_100_000),
        "de volta ao visto, publica",
    );
}

// ---------------------------------------------------------------------------
// (3) fechamento com p_ciclo ≠ gaveta → recusa
// ---------------------------------------------------------------------------

#[test]
fn fechamento_com_p_diferente_da_gaveta_e_recusado() {
    let mut env = Env::new();
    let _c = env.cotista(5_000 * E6);
    env.p_na_gaveta(1_000 * E6);
    env.publish_nav_pela_mesa(1_200_000);
    let authority = env.authority.insecure_clone();
    let gaveta = env.gaveta;

    // a proposta votou 999, a gaveta tem 1.000 (alguem mandou entre a proposta e a execucao)
    assert_dom_error(
        env.deposit_especial_raw(&authority, gaveta, 999 * E6),
        DomError::PDiferenteDoEsperado,
        "P da proposta menor que a gaveta",
    );
    assert_dom_error(
        env.deposit_especial_raw(&authority, gaveta, 1_001 * E6),
        DomError::PDiferenteDoEsperado,
        "P da proposta maior que a gaveta",
    );
    // o invariante p_ciclo == gaveta vale depois de qualquer sequencia de leituras
    env.p_na_gaveta(250 * E6);
    env.sincronizar_gaveta();
    env.sincronizar_gaveta();
    let v = env.vault();
    assert_eq!(v.p_ciclo, 1_250 * E6);
    assert_eq!(v.gaveta_saldo_visto, saldo(&env.svm, &gaveta));
    assert_dom_error(
        env.deposit_especial_raw(&authority, gaveta, 1_000 * E6),
        DomError::PDiferenteDoEsperado,
        "P velho depois de mais P",
    );
    assert_ok(
        env.deposit_especial_raw(&authority, gaveta, 1_250 * E6),
        "o P certo fecha",
    );
    assert_eq!(env.vault().p_ciclo, 0);
    assert_eq!(env.vault().gaveta_saldo_visto, 0);
}

// ---------------------------------------------------------------------------
// (4) dois fechamentos seguidos sem P → ganho zero, sem pânico numérico
// ---------------------------------------------------------------------------

#[test]
fn fechamentos_seguidos_sem_p_dao_ganho_zero_sem_panico() {
    let mut env = Env::new();
    let a = env.cotista(5_000 * E6);
    let b = env.cotista(3_000 * E6);
    env.p_na_gaveta(800 * E6);
    env.publish_nav_pela_mesa(1_100_000);
    env.deposit_especial(800 * E6);
    let indice_apos_1 = env.vault().indice_ciclo;

    // P = 0: erro nomeado, nao panico
    env.avancar(DISTRIBUICAO_INTERVAL);
    let authority = env.authority.insecure_clone();
    let gaveta = env.gaveta;
    assert_dom_error(
        env.deposit_especial_raw(&authority, gaveta, 0),
        DomError::ZeroLucro,
        "fechamento sem P",
    );

    // P = 1 micro: fecha, ganho zero para todos, mesa zero, sem panico
    env.p_na_gaveta(1);
    env.publish_nav_pela_mesa(1_100_000);
    assert_ok(
        env.deposit_especial_raw(&authority, gaveta, 1),
        "fechamento com 1 micro",
    );
    let v = env.vault();
    assert_eq!(v.indice_ciclo_anterior, indice_apos_1);
    assert!(
        v.indice_ciclo >= indice_apos_1 && v.indice_ciclo - indice_apos_1 <= 1_000_000_000_000,
        "o indice andou no maximo 1 micro por cota"
    );
    for h in [&a, &b] {
        let pos = env.posicao_de(&h.wallet.pubkey()).unwrap();
        assert_eq!(
            ganho(
                env.saldo_dom(h),
                pos.indice_entrada,
                v.indice_ciclo_anterior,
                v.indice_ciclo
            )
            .unwrap(),
            0,
            "ganho zero no ciclo vazio"
        );
        assert_dom_error(
            env.sacar_lucro_raw(h),
            DomError::SemLucroASacar,
            "nada a sacar",
        );
        assert_ok(env.acertar_raw(h, true), "acerto do ciclo vazio nao panica");
    }
    // e de novo, 1 micro
    env.avancar(DISTRIBUICAO_INTERVAL);
    env.p_na_gaveta(1);
    env.publish_nav_pela_mesa(1_100_000);
    assert_ok(
        env.deposit_especial_raw(&authority, gaveta, 1),
        "segundo fechamento com 1 micro",
    );
    // o ciclo seguinte, normal, continua funcionando
    env.avancar(DISTRIBUICAO_INTERVAL);
    env.p_na_gaveta(400 * E6);
    env.publish_nav_pela_mesa(1_150_000);
    assert_ok(
        env.deposit_especial_raw(&authority, gaveta, 400 * E6),
        "fechamento normal depois dos vazios",
    );
    assert!(env.vault().lucro_sacavel_restante >= 200 * E6);
}

// ---------------------------------------------------------------------------
// CU de cada instrução tocada pelo J
// ---------------------------------------------------------------------------

const TETO_PADRAO: u64 = 200_000;

#[test]
fn custo_em_cu_das_instrucoes_do_j() {
    let mut env = Env::new();
    let mut linhas: Vec<(&str, u64)> = vec![];

    // 18 carteiras + P (o cenario da mesa), para o fechamento medir no pior caso realista
    let levas: [(&[u64], u64); 6] = [
        (&[55_000, 20_000, 12_000, 9_000], 2_000),
        (&[8_000, 6_500, 5_000, 500], 1_500),
        (&[1_500, 800, 300], 2_500),
        (&[15_000, 7_000, 4_000], 1_800),
        (&[2_500, 10_000], 1_200),
        (&[3_000, 6_000], 1_000),
    ];
    let mut carteiras = vec![];
    let mut capital = 0u64;
    let mut gaveta = 0u64;
    for (aportes, p) in levas {
        let supply = env.supply_dom();
        let nav = if supply == 0 {
            1_000_000
        } else {
            ((capital as u128 + gaveta as u128) * E6 as u128 / supply as u128) as u64
        };
        let meta = env.publish_nav_pela_mesa(nav);
        linhas.push((
            "publish_nav (absorve a gaveta)",
            meta.compute_units_consumed,
        ));
        for usd in aportes.iter() {
            let h = env.carteira_de(Keypair::new(), usd * E6);
            let meta = env.deposit(&h, usd * E6);
            linhas.push((
                "deposit (com acerto e posicao)",
                meta.compute_units_consumed,
            ));
            capital += usd * E6;
            carteiras.push(h);
        }
        env.p_na_gaveta(p * E6);
        gaveta += p * E6;
        let meta = env.sincronizar_gaveta();
        linhas.push(("sincronizar_gaveta", meta.compute_units_consumed));
    }
    let doador = env.carteira(1_000 * E6);
    let meta = assert_ok(
        env.deposit_para_raw(&doador, &carteiras[0], 500 * E6),
        "deposit_para",
    );
    linhas.push(("deposit_para", meta.compute_units_consumed));

    // fechamento com 3 afiliados
    let af1 = env.carteira(0);
    let af2 = env.carteira(0);
    let lista = vec![
        (
            AfiliadoDoFechamento {
                indicado: carteiras[1].wallet.pubkey(),
                afiliado: af1.wallet.pubkey(),
                bps: 500,
            },
            carteiras[1].dom,
            af1.dom,
        ),
        (
            AfiliadoDoFechamento {
                indicado: carteiras[4].wallet.pubkey(),
                afiliado: af2.wallet.pubkey(),
                bps: 500,
            },
            carteiras[4].dom,
            af2.dom,
        ),
        (
            AfiliadoDoFechamento {
                indicado: carteiras[17].wallet.pubkey(),
                afiliado: af2.wallet.pubkey(),
                bps: 300,
            },
            carteiras[17].dom,
            af2.dom,
        ),
    ];
    let supply = env.supply_dom();
    let nav = ((capital as u128 + 500 * E6 as u128 + gaveta as u128) * E6 as u128 / supply as u128)
        as u64;
    env.publish_nav_pela_mesa(nav);
    let meta = env.deposit_especial_afiliados(gaveta, &lista);
    linhas.push((
        "deposit_especial (3 socios + 3 afiliados)",
        meta.compute_units_consumed,
    ));

    // acerto: debito (dono assina) e credito (sem assinatura)
    let meta = env.acertar(&carteiras[0]);
    linhas.push(("acertar (debito, dono assina)", meta.compute_units_consumed));
    let meta = assert_ok(env.acertar_raw(&carteiras[17], false), "credito");
    linhas.push((
        "acertar (credito, sem assinatura)",
        meta.compute_units_consumed,
    ));
    let meta = env.sacar_lucro(&carteiras[2]);
    linhas.push(("sacar_lucro (com acerto)", meta.compute_units_consumed));
    let socio = Holder {
        wallet: env.socios[0].wallet.insecure_clone(),
        dom: env.socios[0].dom,
        usdc: env.socios[0].usdc,
    };
    let fee = env.ledger(&socio.wallet.pubkey()).shares;
    let meta = env.redeem_fee_share(&socio, fee);
    linhas.push(("redeem_fee_share (com acerto)", meta.compute_units_consumed));

    // fora da janela: transferencia com hook e resgate composto
    env.fechar_janela_de_lucro();
    env.acertar(&carteiras[3]);
    env.acertar(&carteiras[5]);
    let meta = assert_ok(
        env.transfer(
            &carteiras[3],
            &carteiras[5].dom,
            &carteiras[5].wallet.pubkey(),
            100 * E6,
        ),
        "transfer",
    );
    linhas.push((
        "transfer_checked + hook (posicoes)",
        meta.compute_units_consumed,
    ));
    let id_antes = env.vault().proximo_pedido_id;
    let meta = assert_ok(
        env.solicitar_resgate_raw(&carteiras[6], 200 * E6),
        "resgate",
    );
    assert_eq!(env.vault().proximo_pedido_id, id_antes + 1);
    linhas.push((
        "acertar + transfer(escrow) + solicitar_resgate",
        meta.compute_units_consumed,
    ));

    println!("\n  CU por instrucao (Upgrade J) — teto padrao {TETO_PADRAO} por transacao");
    let mut pior: std::collections::BTreeMap<&str, u64> = Default::default();
    for (nome, cu) in &linhas {
        let e = pior.entry(nome).or_insert(0);
        if cu > e {
            *e = *cu;
        }
    }
    for (nome, cu) in &pior {
        println!(
            "  {:<48} {:>8} CU  sobra {:>3}%",
            nome,
            cu,
            100 - cu * 100 / TETO_PADRAO
        );
        assert!(
            *cu < TETO_PADRAO * 85 / 100,
            "{nome} consome {cu} CU — menos de 15% de margem no teto padrao"
        );
    }
}
