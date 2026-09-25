//! **O gabarito definitivo dos 18** (D-F2-43 §2), gerado das funções puras do
//! contrato — `absorver_gaveta`, `entrada_ponderada`, `ganho`,
//! `shares_from_usdc` — em inteiros (micro-USDC, índice × 1e18). Substitui a
//! tabela em float da decisão.
//!
//! O arquivo `tests/fixtures/gabarito-18-carteiras.json` é o mesmo que vive em
//! `dom-arquitetura/gabarito-18-carteiras.json`: a suíte Rust (aqui e em
//! `indice_j.rs`, contra a cadeia) e a suíte TS do backend leem o mesmo
//! arquivo. Para regerar: `GABARITO_ESCREVER=1 cargo test --test gabarito`.
//! Sem a variável, o teste confere que o arquivo bate com as funções — o
//! gabarito não envelhece em silêncio.
#![allow(clippy::result_large_err)]

use {
    dom_vault::{
        constants::{INDICE_SCALE, NAV_GENESIS, PERF_FEE_BPS_TOTAL},
        indice::{absorver_gaveta, entrada_ponderada, ganho},
        math::shares_from_usdc,
    },
    serde_json::{json, Value},
};

const E6: u64 = 1_000_000;
const ARQUIVO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/gabarito-18-carteiras.json"
);

/// A sequência da mesa: seis levas de aportes, cada uma seguida de um P.
pub const LEVAS: [(&[(&str, u64)], u64); 6] = [
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

struct Carteira {
    nome: &'static str,
    cotas: u64,
    entrada: u128,
}

/// Roda a sequência com as funções do contrato e devolve o JSON do gabarito.
pub fn gerar() -> Value {
    let mut carteiras: Vec<Carteira> = vec![];
    let mut supply: u64 = 0;
    let mut capital: u64 = 0;
    let mut gaveta: u64 = 0;
    let mut saldo_visto: u64 = 0;
    let mut indice_p: u128 = 0;
    let mut levas_json = vec![];

    for (aportes, p) in LEVAS {
        // o oráculo publica o piso antes da leva: (capital + gaveta) ÷ supply
        let nav = if supply == 0 {
            NAV_GENESIS
        } else {
            ((capital as u128 + gaveta as u128) * E6 as u128 / supply as u128) as u64
        };
        let mut aportes_json = vec![];
        for (nome, usd) in aportes.iter() {
            let usd_micro = usd * E6;
            let cotas = shares_from_usdc(usd_micro, nav).unwrap();
            // abrir_posicao nasce em indice_p; o primeiro lote entra com cotas_antes = 0
            let entrada = entrada_ponderada(0, indice_p, cotas, indice_p).unwrap();
            supply += cotas;
            capital += usd_micro;
            carteiras.push(Carteira {
                nome,
                cotas,
                entrada,
            });
            aportes_json.push(json!({
                "nome": nome, "usd_micro": usd_micro, "nav_micro": nav,
                "cotas_micro": cotas, "indice_entrada": entrada.to_string(),
            }));
        }
        // o P cai na gaveta e é absorvido (sincronizar_gaveta / publish_nav)
        gaveta += p * E6;
        let a = absorver_gaveta(gaveta, saldo_visto, supply, indice_p).unwrap();
        saldo_visto = gaveta;
        indice_p = a.indice_p;
        levas_json.push(json!({
            "nav_micro": nav, "aportes": aportes_json, "p_micro": p * E6,
            "supply_depois_micro": supply, "indice_p_depois": indice_p.to_string(),
        }));
    }

    let taxa = PERF_FEE_BPS_TOTAL as u128;
    let mut soma: u64 = 0;
    let carteiras_json: Vec<Value> = carteiras
        .iter()
        .map(|c| {
            let g = ganho(c.cotas, c.entrada, 0, indice_p).unwrap();
            soma += g;
            // As duas partes sao PISO, cada uma pela sua porta: a mesa e' o que o
            // acerto cobra (`devido = ganho × taxa ÷ 10000`, indice.rs) e a do
            // cotista e' o que a janela paga (`ganho × (10000 − taxa) ÷ 10000`,
            // sacar_lucro.rs). Com ganho impar sobra 1 micro, que fica no cofre.
            let mesa = (g as u128 * taxa / 10_000) as u64;
            let cotista = (g as u128 * (10_000 - taxa) / 10_000) as u64;
            json!({
                "nome": c.nome, "cotas_micro": c.cotas, "indice_entrada": c.entrada.to_string(),
                "ganho_micro": g, "parte_mesa_micro": mesa, "parte_cotista_micro": cotista,
                "sobra_no_cofre_micro": g - mesa - cotista,
            })
        })
        .collect();

    json!({
        "_fonte": "gerado por programs/dom_vault/tests/gabarito.rs a partir de indice.rs e math.rs — nao editar a mao",
        "regra": "D-F2-43 §2 — indice de P por cota (reward-per-share); partes: mesa = piso(ganho × taxa ÷ 1e4) [o acerto], cotista = piso(ganho × (1e4 − taxa) ÷ 1e4) [a janela]; a sobra de 1 micro fica no cofre",
        "indice_scale": INDICE_SCALE.to_string(),
        "taxa_mesa_bps": PERF_FEE_BPS_TOTAL,
        "nav_genesis_micro": NAV_GENESIS,
        "p_total_micro": gaveta,
        "supply_final_micro": supply,
        "capital_micro": capital,
        "indice_p_final": indice_p.to_string(),
        "levas": levas_json,
        "carteiras": carteiras_json,
        "soma_ganho_micro": soma,
        "residuo_micro": gaveta - soma,
    })
}

#[test]
fn o_gabarito_conserva_o_p_e_bate_com_o_arquivo() {
    let g = gerar();
    let soma = g["soma_ganho_micro"].as_u64().unwrap();
    let p = g["p_total_micro"].as_u64().unwrap();
    assert!(
        p - soma < 18,
        "Σ ganho {soma} vs P {p}: o resíduo é só o truncamento (< 1 micro por carteira)"
    );
    // os números da mesa (float, ±1 centavo) — a tabela da D-F2-43 §2
    let de = |n: &str| {
        g["carteiras"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["nome"] == n)
            .unwrap()["ganho_micro"]
            .as_u64()
            .unwrap()
    };
    assert!(de("Egnon").abs_diff(4_482_330_000) <= 10_000);
    assert!(de("Filipe").abs_diff(1_629_940_000) <= 10_000);
    assert!(de("Rosane").abs_diff(29_710_000) <= 10_000);
    assert!(de("Bruno").abs_diff(34_270_000) <= 10_000);

    let texto = serde_json::to_string_pretty(&g).unwrap() + "\n";
    if std::env::var("GABARITO_ESCREVER").is_ok() {
        std::fs::write(ARQUIVO, &texto).unwrap();
        println!("gabarito escrito em {ARQUIVO}");
        return;
    }
    let no_disco = std::fs::read_to_string(ARQUIVO)
        .expect("tests/fixtures/gabarito-18-carteiras.json ausente — gere com GABARITO_ESCREVER=1");
    assert_eq!(no_disco, texto, "o gabarito no disco nao bate com as funcoes do contrato — regere com GABARITO_ESCREVER=1 e leve ao acervo");
}
