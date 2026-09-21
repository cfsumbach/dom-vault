//! Upgrade J — a simulação da mesa (D-F2-43 §2) em LiteSVM, e a prova que o
//! J7 existe para passar: **o valor de cada carteira depois do fechamento cai
//! exatamente `ganho_i × taxa_mesa`**, não `cotas_i × taxa_mesa × P ÷ supply`.
//!
//! Sequência: Egnon 55k, Filipe 20k, Leonardo 12k, Djan 9k → P 2.000 → Ariel 8k,
//! Gisleno 6,5k, Silvio 5k, Rosane 500 → P 1.500 → Raony 1,5k, Abraão 800,
//! Hugo 300 → P 2.500 → Edson 15k, Ismar 7k, Rafael 4k → P 1.800 → Victor 2,5k,
//! Marcos 10k → P 1.200 → Tatiane 3k, Bruno 6k → P 1.000 → fechamento.
//!
//! O NAV de aporte é o piso (capital + gaveta) ÷ supply, publicado pelo oráculo
//! antes de cada leva — sem marcação, como na simulação da mesa.
mod common;

use {
    anchor_lang::prelude::Pubkey,
    common::*,
    dom_vault::{constants::*, error::DomError, indice::ganho, state::AfiliadoDoFechamento},
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const E6: u64 = 1_000_000;

struct Carteira {
    nome: &'static str,
    holder: Holder,
    ganho: u64,
}

/// Monta o cofre com as 18 carteiras e o P de 10.000 na gaveta, ABSORVIDO no
/// índice (sem fechar). Devolve as carteiras e o supply.
fn cenario_da_mesa(env: &mut Env) -> Vec<Carteira> {
    let levas: [(&[(&str, u64)], u64); 6] = [
        (
            &[
                ("Egnon", 55_000),
                ("Filipe", 20_000),
                ("Leonardo", 12_000),
                ("Djan", 9_000),
            ],
            2_000,
        ),
        (
            &[
                ("Ariel", 8_000),
                ("Gisleno", 6_500),
                ("Silvio", 5_000),
                ("Rosane", 500),
            ],
            1_500,
        ),
        (&[("Raony", 1_500), ("Abraao", 800), ("Hugo", 300)], 2_500),
        (
            &[("Edson", 15_000), ("Ismar", 7_000), ("Rafael", 4_000)],
            1_800,
        ),
        (&[("Victor", 2_500), ("Marcos", 10_000)], 1_200),
        (&[("Tatiane", 3_000), ("Bruno", 6_000)], 1_000),
    ];
    let mut carteiras = vec![];
    let mut capital: u64 = 0;
    let mut gaveta: u64 = 0;
    for (aportes, p) in levas {
        // o oráculo publica o piso antes da leva (a cadência é ignorada: a mesa publica pela válvula)
        let supply = env.supply_dom();
        let nav = if supply == 0 {
            NAV_GENESIS
        } else {
            ((capital as u128 + gaveta as u128) * E6 as u128 / supply as u128) as u64
        };
        env.publish_nav_pela_mesa(nav);
        for (nome, usd) in aportes.iter() {
            let holder = env.carteira_de(Keypair::new(), usd * E6);
            env.deposit(&holder, usd * E6);
            capital += usd * E6;
            carteiras.push(Carteira {
                nome,
                holder,
                ganho: 0,
            });
        }
        env.p_na_gaveta(p * E6);
        gaveta += p * E6;
        env.sincronizar_gaveta();
    }
    assert_eq!(gaveta, 10_000 * E6);
    let v = env.vault();
    for c in carteiras.iter_mut() {
        let pos = env
            .posicao_de(&c.holder.wallet.pubkey())
            .expect("posicao existe");
        c.ganho = ganho(
            env.saldo_dom(&c.holder),
            pos.indice_entrada,
            v.indice_ciclo,
            v.indice_p,
        )
        .unwrap();
    }
    carteiras
}

#[test]
fn a_simulacao_da_mesa_na_cadeia_conserva_o_p() {
    let mut env = Env::new();
    let carteiras = cenario_da_mesa(&mut env);
    let soma: u64 = carteiras.iter().map(|c| c.ganho).sum();
    assert!(10_000 * E6 - soma < 18, "Σ ganho {soma} vs P 10.000");

    // O gabarito definitivo (tests/fixtures/gabarito-18-carteiras.json, gerado
    // das funcoes puras em tests/gabarito.rs): a cadeia tem de bater AO MICRO —
    // cotas, entrada e ganho de cada carteira, e o indice final.
    let gabarito: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/gabarito-18-carteiras.json")).unwrap();
    let v = env.vault();
    assert_eq!(
        v.indice_p.to_string(),
        gabarito["indice_p_final"].as_str().unwrap(),
        "indice_p final"
    );
    assert_eq!(
        env.supply_dom(),
        gabarito["supply_final_micro"].as_u64().unwrap(),
        "supply final"
    );
    for (c, g) in carteiras
        .iter()
        .zip(gabarito["carteiras"].as_array().unwrap())
    {
        assert_eq!(c.nome, g["nome"].as_str().unwrap());
        assert_eq!(
            env.saldo_dom(&c.holder),
            g["cotas_micro"].as_u64().unwrap(),
            "{}: cotas",
            c.nome
        );
        let pos = env.posicao_de(&c.holder.wallet.pubkey()).unwrap();
        assert_eq!(
            pos.indice_entrada.to_string(),
            g["indice_entrada"].as_str().unwrap(),
            "{}: entrada",
            c.nome
        );
        assert_eq!(
            c.ganho,
            g["ganho_micro"].as_u64().unwrap(),
            "{}: ganho",
            c.nome
        );
    }
    assert_eq!(soma, gabarito["soma_ganho_micro"].as_u64().unwrap());
    assert_eq!(v.p_ciclo, 10_000 * E6, "p_ciclo soma os deltas da gaveta");
    assert_eq!(v.gaveta_saldo_visto, 10_000 * E6);
}

/// J7: depois do fechamento, cada carteira vale `antes − ganho_i × taxa_mesa`.
///
/// Com a cunhagem uniforme do J4 isto FALHA (Bruno perde ~167 devendo 17;
/// Egnon perde ~1.690 devendo 2.241). O J7 existe para este teste passar.
#[test]
fn j7_o_valor_de_cada_carteira_cai_exatamente_o_ganho_vezes_a_taxa() {
    let mut env = Env::new();
    let carteiras = cenario_da_mesa(&mut env);
    let taxa_bps = env.vault().perf_fee_bps_total as u128;

    // o NAV do fechamento: piso com o P dentro
    let supply_antes = env.supply_dom();
    let capital: u64 = 166_100 * E6;
    let nav_antes =
        ((capital as u128 + 10_000 * E6 as u128) * E6 as u128 / supply_antes as u128) as u64;
    env.publish_nav_pela_mesa(nav_antes);
    let valor_antes: Vec<u64> = carteiras
        .iter()
        .map(|c| (env.saldo_dom(&c.holder) as u128 * nav_antes as u128 / E6 as u128) as u64)
        .collect();

    // fechamento (J2: o P sai da gaveta; §4: o NAV publicado nao muda)
    env.deposit_especial(10_000 * E6);
    let supply_pos_fechamento = env.supply_dom();
    let cotas_socios = supply_pos_fechamento - supply_antes;
    // toca cada carteira para o acerto (J7): creditos cunham, debitos queimam (o dono assina)
    for c in &carteiras {
        env.acertar(&c.holder);
    }
    // conservacao: Σ(pago − devido) = 0 → o supply volta ao de antes + a mesa (ate' 1 cota de truncamento por carteira)
    let supply_final = env.supply_dom();
    assert!(
        supply_final.abs_diff(supply_pos_fechamento) <= carteiras.len() as u64 * 2,
        "supply {supply_final} vs {supply_pos_fechamento} (socios {cotas_socios})"
    );
    // o NAV economico depois da cunhagem: o oraculo publica (capital + P) ÷ supply na proxima hora
    let nav_depois =
        ((capital as u128 + 10_000 * E6 as u128) * E6 as u128 / supply_final as u128) as u64;
    assert_eq!(
        env.vault().nav,
        nav_antes,
        "§4: o fechamento nao mexe no NAV publicado"
    );
    let mut erros = vec![];
    for (i, c) in carteiras.iter().enumerate() {
        let valor_depois =
            (env.saldo_dom(&c.holder) as u128 * nav_depois as u128 / E6 as u128) as u64;
        let devido = (c.ganho as u128 * taxa_bps / 10_000) as u64;
        let esperado = valor_antes[i] - devido;
        if valor_depois.abs_diff(esperado) > 20_000 {
            // dois centavos: truncamentos de cota e de indice
            erros.push(format!(
                "{}: antes {:.2} depois {:.2} devido {:.2} esperado {:.2}",
                c.nome,
                valor_antes[i] as f64 / 1e6,
                valor_depois as f64 / 1e6,
                devido as f64 / 1e6,
                esperado as f64 / 1e6
            ));
        }
    }
    assert!(
        erros.is_empty(),
        "a cobranca nao seguiu o indice:\n{}",
        erros.join("\n")
    );
}

/// J1: três afiliados sobre a simulação da mesa. A comissão de cada um é
/// `ganho_indicado × taxa_mesa × bps` e sai DE DENTRO da mesa: os sócios
/// recebem `(mesa − Σ comissões) ÷ 3`, e o valor de cada cotista cai
/// exatamente `ganho_i × taxa_mesa` — como sem afiliado nenhum.
///
/// Afiliados: A indicou Filipe (5%), B indicou Ariel e Bruno (5% e 3%).
#[test]
fn j1_a_comissao_sai_da_mesa_e_o_cotista_nao_paga_um_micro_a_mais() {
    let mut env = Env::new();
    let carteiras = cenario_da_mesa(&mut env);
    let taxa_bps = env.vault().perf_fee_bps_total as u128;
    let de = |n: &str| carteiras.iter().find(|c| c.nome == n).unwrap();

    // afiliados: carteiras aprovadas, com posicao, sem cota
    let a = env.carteira(0);
    let b = env.carteira(0);
    let lista = vec![
        (
            AfiliadoDoFechamento {
                indicado: de("Filipe").holder.wallet.pubkey(),
                afiliado: a.wallet.pubkey(),
                bps: 500,
            },
            de("Filipe").holder.dom,
            a.dom,
        ),
        (
            AfiliadoDoFechamento {
                indicado: de("Ariel").holder.wallet.pubkey(),
                afiliado: b.wallet.pubkey(),
                bps: 500,
            },
            de("Ariel").holder.dom,
            b.dom,
        ),
        (
            AfiliadoDoFechamento {
                indicado: de("Bruno").holder.wallet.pubkey(),
                afiliado: b.wallet.pubkey(),
                bps: 300,
            },
            de("Bruno").holder.dom,
            b.dom,
        ),
    ];
    let esperado_a = (de("Filipe").ganho as u128 * taxa_bps * 500 / 100_000_000) as u64;
    let esperado_b = (de("Ariel").ganho as u128 * taxa_bps * 500 / 100_000_000) as u64
        + (de("Bruno").ganho as u128 * taxa_bps * 300 / 100_000_000) as u64;

    let supply_antes = env.supply_dom();
    let capital: u64 = 166_100 * E6;
    let nav_antes =
        ((capital as u128 + 10_000 * E6 as u128) * E6 as u128 / supply_antes as u128) as u64;
    env.publish_nav_pela_mesa(nav_antes);
    let valor_antes: Vec<u64> = carteiras
        .iter()
        .map(|c| (env.saldo_dom(&c.holder) as u128 * nav_antes as u128 / E6 as u128) as u64)
        .collect();
    let socio_antes = saldo(&env.svm, &env.socios[0].dom);

    env.deposit_especial_afiliados(10_000 * E6, &lista);
    let v = env.vault();
    let nav_f = v.nav_fechamento as u128;

    // as comissoes, em USDC ao nav_fechamento (± 1 micro de truncamento de cota)
    let usdc = |cotas: u64| (cotas as u128 * nav_f / E6 as u128) as u64;
    assert!(
        usdc(env.saldo_dom(&a)).abs_diff(esperado_a) <= 2,
        "A: {} vs {esperado_a}",
        usdc(env.saldo_dom(&a))
    );
    assert!(
        usdc(env.saldo_dom(&b)).abs_diff(esperado_b) <= 3,
        "B: {} vs {esperado_b}",
        usdc(env.saldo_dom(&b))
    );
    assert_eq!(
        env.posicao_de(&a.wallet.pubkey()).unwrap().indice_entrada,
        v.indice_p,
        "a comissao entra ao indice de agora"
    );

    // os socios: (mesa − Σ comissoes) ÷ 3, cada
    let mesa = 10_000 * E6 as u128 * taxa_bps / 10_000;
    let por_socio = (mesa - esperado_a as u128 - esperado_b as u128) / 3;
    let socio_recebeu = usdc(saldo(&env.svm, &env.socios[0].dom) - socio_antes);
    assert!(
        (socio_recebeu as u128).abs_diff(por_socio) <= 2,
        "socio: {socio_recebeu} vs {por_socio}"
    );
    assert_eq!(
        env.ledger(&env.socios[0].wallet.pubkey()).shares,
        saldo(&env.svm, &env.socios[0].dom) - socio_antes
    );
    // o bolo dos cotistas nao mudou: P − mesa (+ sobra)
    assert!((v.lucro_sacavel_restante as u128).abs_diff(10_000 * E6 as u128 - mesa) <= 2);

    // e cada cotista paga exatamente o que pagaria sem afiliado
    for c in &carteiras {
        env.acertar(&c.holder);
    }
    let supply_final = env.supply_dom();
    let nav_depois =
        ((capital as u128 + 10_000 * E6 as u128) * E6 as u128 / supply_final as u128) as u64;
    let mut erros = vec![];
    for (i, c) in carteiras.iter().enumerate() {
        let valor_depois =
            (env.saldo_dom(&c.holder) as u128 * nav_depois as u128 / E6 as u128) as u64;
        let devido = (c.ganho as u128 * taxa_bps / 10_000) as u64;
        if valor_depois.abs_diff(valor_antes[i] - devido) > 20_000 {
            erros.push(format!(
                "{}: antes {} depois {} devido {}",
                c.nome, valor_antes[i], valor_depois, devido
            ));
        }
    }
    assert!(
        erros.is_empty(),
        "o cotista pagou diferente com afiliado:\n{}",
        erros.join("\n")
    );
}

/// J1: a lista é conferida — bps fora da faixa, indicado repetido, contas
/// que não batem com a lista.
#[test]
fn j1_lista_invalida_e_recusada() {
    let mut env = Env::new();
    let cotista = env.cotista(5_000 * E6);
    let a = env.carteira(0);
    let outro = env.cotista(1_000 * E6);
    env.publish_nav_pela_mesa(1_200_000);
    let authority = env.authority.insecure_clone();
    let (socios, contas): (Vec<Pubkey>, Vec<Pubkey>) = (
        env.socios.iter().map(|s| s.wallet.pubkey()).collect(),
        env.socios.iter().map(|s| s.dom).collect(),
    );
    let gaveta = env.gaveta;
    env.p_na_gaveta(1_000 * E6);
    let linha = |bps: u16| AfiliadoDoFechamento {
        indicado: cotista.wallet.pubkey(),
        afiliado: a.wallet.pubkey(),
        bps,
    };

    assert_dom_error(
        env.deposit_especial_com_afiliados(
            &authority,
            gaveta,
            1_000 * E6,
            &contas,
            &socios,
            &[(linha(0), cotista.dom, a.dom)],
        ),
        DomError::AfiliadoInvalido,
        "bps zero",
    );
    assert_dom_error(
        env.deposit_especial_com_afiliados(
            &authority,
            gaveta,
            1_000 * E6,
            &contas,
            &socios,
            &[(linha(501), cotista.dom, a.dom)],
        ),
        DomError::AfiliadoInvalido,
        "bps acima do teto (500 = 5%)",
    );
    assert_dom_error(
        env.deposit_especial_com_afiliados(
            &authority,
            gaveta,
            1_000 * E6,
            &contas,
            &socios,
            &[
                (linha(500), cotista.dom, a.dom),
                (linha(300), cotista.dom, a.dom),
            ],
        ),
        DomError::AfiliadoRepetido,
        "mesmo indicado duas vezes",
    );
    assert_dom_error(
        env.deposit_especial_com_afiliados(
            &authority,
            gaveta,
            1_000 * E6,
            &contas,
            &socios,
            &[(linha(500), outro.dom, a.dom)],
        ),
        DomError::AfiliadoInvalido,
        "conta de cota de outro cotista no lugar do indicado",
    );
    let auto = AfiliadoDoFechamento {
        indicado: cotista.wallet.pubkey(),
        afiliado: cotista.wallet.pubkey(),
        bps: 500,
    };
    assert_dom_error(
        env.deposit_especial_com_afiliados(
            &authority,
            gaveta,
            1_000 * E6,
            &contas,
            &socios,
            &[(auto, cotista.dom, cotista.dom)],
        ),
        DomError::AfiliadoInvalido,
        "indicado == afiliado",
    );
    // contra-teste: a lista boa passa
    assert_ok(
        env.deposit_especial_com_afiliados(
            &authority,
            gaveta,
            1_000 * E6,
            &contas,
            &socios,
            &[(linha(500), cotista.dom, a.dom)],
        ),
        "lista valida",
    );
    // supply 6.000 (5.000 + 1.000): indice 1.000/6.000; ganho do indicado 5.000 × 0,166666 = 833,33;
    // parte da mesa 416,67; comissao 5% = 20,833333 USDC; nav_fechamento 1,2 − 500/6.000 = 1,116666
    // → 18,656716 cotas (± truncamentos de indice e de cota)
    assert!(
        env.saldo_dom(&a).abs_diff(18_656_716) <= 10,
        "comissao {}",
        env.saldo_dom(&a)
    );
}
