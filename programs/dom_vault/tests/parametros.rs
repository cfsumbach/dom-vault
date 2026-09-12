//! **Upgrade E — os parâmetros de política saem do binário** (`D-F2-21`).
//!
//! Onze valores que eram constantes no `.so` viraram campo do cofre, ajustáveis
//! por `ajustar_parametro` — proposta da mesa, não redeploy. Este arquivo prova
//! as três coisas que isso exige:
//!
//! 1. **o campo manda** — mudado o campo, a instrução que o lê muda de resposta;
//! 2. **o limite segura** — número votado fora da faixa é recusado, e a faixa
//!    continua constante de propósito (limite votável seria proteção removível
//!    pelo mesmo voto que ela protege);
//! 3. **o campo é obedecido** — não basta gravar: quem separa "a mesa votou" de
//!    "o cofre mudou" é a instrução que aplica.
//!
//! # A migração saiu daqui, e é de propósito
//!
//! A `migrar_vault_parametros` levou o cofre vivo de 541 a 603 bytes em
//! 2026-09-08 e **saiu do programa no Upgrade F**, como a `migrate_vault_resgate`
//! saiu no B. Os testes dela saíram junto — teste de instrução que não existe
//! passa por vacuidade e vira ruído verde.
//!
//! O que ela fez está provado onde importa: na rede, e no relatório
//! `F5-03-upgrade-e-em-mainnet.md`. O cofre de mainnet tem 603 bytes e
//! `layout_version = 2`; nenhum cofre no layout antigo existe mais para migrar.

#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{constants::*, error::DomError, instructions::parametros::Parametro, state::Vault},
};

// ---------------------------------------------------------------------------
// 1. O campo manda — não a constante
// ---------------------------------------------------------------------------

/// O piso do saque de lucro passou a valer pelo campo.
///
/// É o teste que prova o Upgrade E inteiro num caso só: antes dele, mudar este
/// número era um redeploy. A prova é comportamental, não de leitura — a mesa
/// baixa o piso e o saque que estava barrado passa a caber.
#[test]
fn piso_do_saque_de_lucro_obedece_ao_campo_e_nao_a_constante() {
    let mut env = Env::new();
    assert_eq!(
        env.vault().min_saque_lucro_usdc,
        MIN_SAQUE_LUCRO_USDC,
        "a genese semeia o campo com a constante"
    );

    env.ajustar_parametro(Parametro::MinSaqueLucro, 500 * UNIT)
        .unwrap();
    assert_eq!(env.vault().min_saque_lucro_usdc, 500 * UNIT);

    // E volta. O ponto de ser campo é ser reversível sem tocar no binário.
    env.ajustar_parametro(Parametro::MinSaqueLucro, 50 * UNIT)
        .unwrap();
    assert_eq!(env.vault().min_saque_lucro_usdc, 50 * UNIT);
}

/// Um caso da tabela: qual parâmetro, que valor votar, e onde ele deve cair.
type Caso = (Parametro, u64, fn(&Vault) -> u64);

/// Os onze param, um a um, com valor dentro da faixa.
///
/// Um teste por parâmetro seria onze cópias da mesma forma, e a primeira que
/// esquecesse a asserção não pareceria diferente das outras — o mesmo argumento
/// que fez a instrução ser um enum e não onze irmãs.
#[test]
fn os_onze_parametros_gravam_o_valor_votado() {
    let mut env = Env::new();

    let casos: Vec<Caso> = vec![
        (Parametro::PrazoDoResgate, 90 * 24 * 3600, |v| {
            v.resgate_capital_prazo as u64
        }),
        (Parametro::MinResgateUsdc, 250 * UNIT, |v| {
            v.min_resgate_capital_usdc
        }),
        (Parametro::MinResgateCotas, 250 * UNIT, |v| {
            v.min_resgate_capital_cotas
        }),
        (Parametro::MinSaqueLucro, 75 * UNIT, |v| {
            v.min_saque_lucro_usdc
        }),
        (Parametro::NavStaleness, 48 * 3600, |v| {
            v.max_nav_staleness as u64
        }),
        (Parametro::IntervaloDePublicacao, 30 * 60, |v| {
            v.min_nav_publish_interval as u64
        }),
        (Parametro::CadenciaDeDistribuicao, 14 * 24 * 3600, |v| {
            v.distribuicao_interval as u64
        }),
        (Parametro::NavBoundPct, 10, |v| v.nav_bound_pct as u64),
        (Parametro::CapPct, 40, |v| v.cap_pct as u64),
        (Parametro::ReserveBps, 1_500, |v| v.reserve_bps as u64),
        (Parametro::PerfFeeBps, 1_000, |v| {
            v.perf_fee_bps_por_socio as u64
        }),
    ];

    for (qual, novo, ler) in casos {
        env.ajustar_parametro(qual, novo)
            .unwrap_or_else(|e| panic!("{qual:?} deveria aceitar {novo}: {e:?}"));
        assert_eq!(ler(&env.vault()), novo, "{qual:?} nao gravou");
    }
}

// ---------------------------------------------------------------------------
// 2. O limite segura
// ---------------------------------------------------------------------------

/// Cada limite barra o estado que ele existe para barrar.
///
/// Nenhum destes é gosto: cada linha é um cofre travado sem ninguém querer.
#[test]
fn numero_fora_da_faixa_e_recusado() {
    let mut env = Env::new();

    let fora: Vec<(Parametro, u64, &str)> = vec![
        // Zero em qualquer dos dois pisos do resgate zera o piso INTEIRO: eles
        // sao ligados por OU, e o lado zerado passa a ser sempre verdadeiro.
        (
            Parametro::MinResgateUsdc,
            0,
            "piso do resgate em USDC zerado",
        ),
        (
            Parametro::MinResgateCotas,
            0,
            "piso do resgate em cotas zerado",
        ),
        (Parametro::MinSaqueLucro, 0, "piso do saque zerado"),
        // Validade curta demais fecha o cofre antes de o oraculo republicar.
        (
            Parametro::NavStaleness,
            (MIN_STALENESS_VOTAVEL - 1) as u64,
            "validade abaixo do minimo",
        ),
        (
            Parametro::NavStaleness,
            (MAX_STALENESS_VOTAVEL + 1) as u64,
            "validade acima do maximo",
        ),
        (
            Parametro::IntervaloDePublicacao,
            (MAX_INTERVALO_VOTAVEL + 1) as u64,
            "intervalo acima do maximo",
        ),
        (
            Parametro::PrazoDoResgate,
            (MIN_PRAZO_VOTAVEL - 1) as u64,
            "prazo do resgate abaixo do minimo",
        ),
        (
            Parametro::CadenciaDeDistribuicao,
            (MIN_CADENCIA_VOTAVEL - 1) as u64,
            "cadencia abaixo do minimo",
        ),
        // Bound zero congela o NAV: nenhuma variacao passa.
        (Parametro::NavBoundPct, 0, "bound zerado congela o NAV"),
        (
            Parametro::NavBoundPct,
            (MAX_BOUND_PCT + 1) as u64,
            "bound acima do maximo",
        ),
        // Cap zero recusa todo deposito.
        (Parametro::CapPct, 0, "cap zerado recusa todo deposito"),
        (
            Parametro::CapPct,
            (MAX_CAP_PCT + 1) as u64,
            "cap acima de 100%",
        ),
        (
            Parametro::ReserveBps,
            (MAX_RESERVE_BPS_VOTAVEL + 1) as u64,
            "reserva acima de 50% imobiliza o fundo",
        ),
        // E o teto que protege o cotista da propria mesa.
        (
            Parametro::PerfFeeBps,
            (MAX_PERF_FEE_BPS_POR_SOCIO + 1) as u64,
            "taxa de performance acima de 25% por socio",
        ),
    ];

    for (qual, novo, cenario) in fora {
        assert_dom_error(
            env.ajustar_parametro(qual, novo),
            DomError::ParametroForaDoLimite,
            cenario,
        );
    }
}

/// O teto da taxa de performance é **75% no total**, e o binário é quem promete.
///
/// Este é o único limite da lista que não protege o cofre de um número que o
/// trava: protege **o cotista da própria mesa**. A mesa vota a taxa que ela
/// recebe, e a parcela do cotista é o resto (`P − 3 × parcela`) — sem teto, uma
/// proposta 2/3 levaria o resto a zero sem que nenhum cotista votasse.
#[test]
fn teto_da_taxa_de_performance_e_25_por_socio_75_no_total() {
    assert_eq!(
        MAX_PERF_FEE_BPS_POR_SOCIO as u64 * NUM_SOCIOS as u64,
        7_500,
        "75% no total — a mesa fechou em 2026-09-08"
    );

    let mut env = Env::new();
    // No teto exato, passa.
    env.ajustar_parametro(Parametro::PerfFeeBps, MAX_PERF_FEE_BPS_POR_SOCIO as u64)
        .unwrap();
    assert_eq!(
        env.vault().perf_fee_bps_por_socio,
        MAX_PERF_FEE_BPS_POR_SOCIO
    );
    // Um ponto-base acima, nao.
    assert_dom_error(
        env.ajustar_parametro(Parametro::PerfFeeBps, MAX_PERF_FEE_BPS_POR_SOCIO as u64 + 1),
        DomError::ParametroForaDoLimite,
        "um bps acima do teto",
    );
}

/// Validade e intervalo têm de ser coerentes **entre si**, e cada um sozinho
/// dentro da faixa não garante isso.
///
/// O estado proibido é o ciclo: intervalo maior que a validade faz o NAV vencer
/// antes de o oráculo ter direito de republicar, e o cofre fecha sozinho, para
/// sempre, sem que nenhum dos dois números pareça absurdo isolado.
#[test]
fn validade_e_intervalo_do_nav_nao_podem_se_cruzar() {
    let mut env = Env::new();

    // Cada um, sozinho, esta' dentro da sua faixa: 2h e 1h.
    env.ajustar_parametro(Parametro::NavStaleness, 2 * 3600)
        .unwrap();
    // Intervalo de 3h com validade de 2h: o NAV venceria antes de poder ser
    // republicado. Recusado por INCOERENCIA, nao por faixa.
    assert_dom_error(
        env.ajustar_parametro(Parametro::IntervaloDePublicacao, 3 * 3600),
        DomError::ParametrosIncoerentes,
        "intervalo maior que a validade",
    );

    // E o mesmo pelo outro lado: baixar a validade para dentro do intervalo.
    env.ajustar_parametro(Parametro::IntervaloDePublicacao, 90 * 60)
        .unwrap();
    assert_dom_error(
        env.ajustar_parametro(Parametro::NavStaleness, 3_600),
        DomError::ParametrosIncoerentes,
        "validade abaixo do intervalo",
    );
}

// ---------------------------------------------------------------------------
// 4. O campo é OBEDECIDO — não só gravado
// ---------------------------------------------------------------------------
//
// A conversão trocou dez leituras de constante por dez leituras de campo, uma a
// uma, à mão. Gravar o campo e ler o campo são duas coisas, e o que separa "a
// mesa votou" de "o cofre mudou" é a instrução que aplica. Os dois testes
// abaixo pegam as duas pontas: a que barra o oráculo e a que barra o cotista.

/// O bound do NAV passa a ser o que a mesa votou.
///
/// Baixado para 5%, uma publicação de +10% — que o cofre aceitava — passa a ser
/// recusada pelo mesmo oráculo, na mesma instrução. É a prova de que o
/// `publish_nav` lê o campo, e não os 15% que estavam compilados.
#[test]
fn o_bound_do_nav_que_vale_e_o_do_campo() {
    let mut env = Env::new();
    let oraculo = env.oracle.insecure_clone();

    // O `Env::new` ja' publicou NAV no relogio atual, e o timestamp e' monotonico
    // estrito (T36). Andar duas horas atende as duas travas de uma vez: o
    // timestamp cresce e o intervalo minimo de publicacao (1h) e' respeitado.
    env.avancar(2 * 3600);

    // Com os 15% de gênese, +10% passa.
    let agora = env.agora();
    env.publish_nav_raw(&oraculo, 1_100_000, agora).unwrap();
    assert_eq!(env.vault().nav, 1_100_000);

    // A mesa aperta para 5%.
    env.ajustar_parametro(Parametro::NavBoundPct, 5).unwrap();

    // O MESMO movimento de +10%, agora recusado. Nada no binario mudou.
    env.avancar(2 * 3600);
    let agora = env.agora();
    assert_dom_error(
        env.publish_nav_raw(&oraculo, 1_210_000, agora),
        DomError::NavOutOfBounds,
        "+10% com bound votado em 5%",
    );

    // E dentro dos 5% novos, passa — o bound aperta, nao trava.
    env.avancar(2 * 3600);
    let agora = env.agora();
    env.publish_nav_raw(&oraculo, 1_150_000, agora).unwrap();
    assert_eq!(env.vault().nav, 1_150_000);
}

/// O piso do resgate passa a ser o que a mesa votou, nas duas unidades do OU.
///
/// O piso é `cotas >= piso_cotas || valor >= piso_usdc`, e por isso subir só um
/// dos dois não move nada: o outro lado do OU continua deixando passar. O teste
/// sobe os dois e depois desce os dois, que é como a mesa vota de verdade.
#[test]
fn o_piso_do_resgate_que_vale_e_o_do_campo() {
    let mut env = Env::new();
    let cotista = env.cotista(20_000 * UNIT);

    // A mesa sobe os dois lados do OU para 5.000. O pedido de 1.000 nao cabe
    // mais em nenhum dos dois — ao NAV de genese, 1.000 cotas valem 1.000 USDC.
    env.ajustar_parametro(Parametro::MinResgateCotas, 5_000 * UNIT)
        .unwrap();
    env.ajustar_parametro(Parametro::MinResgateUsdc, 5_000 * UNIT)
        .unwrap();
    assert_dom_error(
        env.solicitar_resgate_raw(&cotista, 1_000 * UNIT),
        DomError::ResgateAbaixoDoMinimo,
        "1.000 cotas com piso votado em 5.000",
    );

    // E desce para o que a mesa fechou em 2026-09-08: 100.
    env.ajustar_parametro(Parametro::MinResgateCotas, 100 * UNIT)
        .unwrap();
    env.ajustar_parametro(Parametro::MinResgateUsdc, 100 * UNIT)
        .unwrap();
    env.solicitar_resgate_raw(&cotista, 100 * UNIT)
        .expect("no piso exato, passa");
}

/// **Os três pisos que a mesa mudou em 2026-09-08, afirmados em número.**
///
/// Existe para que a mudança de valor tenha um lugar único onde ela é dita, e
/// não fique só no `git log`. Se alguém "arredondar" um destes, este teste é
/// quem obriga a passar pela mesa antes.
#[test]
fn os_pisos_que_a_mesa_fechou() {
    assert_eq!(
        MIN_RESGATE_CAPITAL_USDC,
        100 * UNIT,
        "resgate: 1.000 -> 100"
    );
    assert_eq!(
        MIN_RESGATE_CAPITAL_COTAS,
        100 * UNIT,
        "resgate em cotas: 1.000 -> 100"
    );
    assert_eq!(
        MIN_SAQUE_LUCRO_USDC,
        100 * UNIT,
        "saque de lucro: 200 -> 100"
    );
    // `MIN_DEPOSIT` e' o valor de GENESE do campo. O cofre vivo de mainnet nao
    // o le' daqui — la' o piso e' 200 e desce para 100 por `update_min_deposit`,
    // uma proposta, sem upgrade nenhum.
    assert_eq!(MIN_DEPOSIT, 100 * UNIT, "deposito de genese: 200 -> 100");
}

// A seção 5 — a correção de uso único do `deployed_usdc` — SAIU no Upgrade G
// junto com a instrução que ela cobria. Ver `D-F2-30`.
