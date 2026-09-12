//! Grupo E da matriz — **NAV** (T35–T37).
//!
//! `publish_nav` é a única instrução que move o NAV. Em F1 o oráculo é uma
//! chave de script; o backend de verdade é F3 (R5 do runbook). O que já é
//! definitivo é o formato: oráculo, hash de atestação e relógio monotônico.

#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{constants::NAV_GENESIS, error::DomError, events::NavPublished},
    solana_keypair::Keypair,
    solana_signer::Signer,
};

// ---------------------------------------------------------------------------
// T35 — só o oráculo publica
// ---------------------------------------------------------------------------

/// Nem a autoridade do cofre publica NAV. São papéis distintos: governança
/// aprova, oráculo carimba. Confundir os dois faria toda publicação de NAV
/// exigir o multisig — e NAV é rotina, não ato de governança.
#[test]
fn t35_publish_nav_por_nao_oraculo_rejeita() {
    let mut env = Env::sem_nav_publicado();
    let agora = env.agora();

    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), SOL).unwrap();
    assert_dom_error(
        env.publish_nav_raw(&intruso, 1_100_000, agora),
        DomError::NotOracle,
        "T35 intruso publicando",
    );

    assert_eq!(env.vault().nav, NAV_GENESIS, "NAV nao mudou");

    // Contra-teste: o oráculo publica e passa.
    let oracle = env.oracle.insecure_clone();
    assert_ok(
        env.publish_nav_raw(&oracle, 1_100_000, agora),
        "T35 contra-teste: oraculo publicando",
    );
    assert_eq!(env.vault().nav, 1_100_000);

    // A autoridade **tambem** passa, desde a valvula do DV11: publicar e' ato de
    // oraculo OU de governanca. O que continua barrado e' o terceiro sem papel
    // nenhum, la' em cima.
    env.avancar_sem_oraculo(1);
    let depois = env.agora();
    let authority = env.authority.insecure_clone();
    assert_ok(
        env.publish_nav_raw(&authority, 1_150_000, depois),
        "T35 autoridade publicando",
    );
    assert_eq!(env.vault().nav, 1_150_000, "a mesa publicou");
}

// ---------------------------------------------------------------------------
// T36 — o relógio não anda para trás
// ---------------------------------------------------------------------------

/// Timestamp regressivo reabriria para trás a janela do "pior valor" e deixaria
/// escolher o instante mais conveniente para carimbar. Igual ao anterior também
/// rejeita: monotônico estrito.
#[test]
fn t36_publish_nav_com_timestamp_regressivo_rejeita() {
    let mut env = Env::sem_nav_publicado();
    let oracle = env.oracle.insecure_clone();
    let t = env.agora();

    assert_ok(
        env.publish_nav_raw(&oracle, 1_100_000, t),
        "T36 primeiro NAV",
    );

    assert_dom_error(
        env.publish_nav_raw(&oracle, 1_200_000, t - 1),
        DomError::NavTimestampRegressive,
        "T36 timestamp anterior",
    );
    assert_dom_error(
        env.publish_nav_raw(&oracle, 1_200_000, t),
        DomError::NavTimestampRegressive,
        "T36 timestamp igual",
    );
    assert_eq!(env.vault().nav, 1_100_000, "NAV nao mudou");

    // Contra-teste: um segundo à frente passa — depois do intervalo mínimo, que
    // é outra trava e tem teste próprio logo abaixo.
    env.avancar_sem_oraculo(dom_vault::constants::MIN_NAV_PUBLISH_INTERVAL);
    assert_ok(
        env.publish_nav_raw(&oracle, 1_200_000, t + 1),
        "T36 contra-teste: timestamp posterior",
    );
    assert_eq!(env.vault().nav, 1_200_000);
}

/// **O intervalo mínimo entre publicações do oráculo.**
///
/// O limite de 15% por publicação protege contra o dedo gordo de um operador
/// honesto. Ele **não** protegia contra quem tem a chave: sem intervalo, a
/// monotonicidade só exige `timestamp > nav_ts`, e o relógio da rede tem
/// granularidade de um segundo. Teto: uma publicação por segundo.
///
/// ```text
/// 0,85^5 = 0,4437   cinco segundos levavam o NAV a menos da metade
/// 1,15^5 = 2,0114   cinco segundos dobravam o NAV
/// ```
///
/// **O limite de 15% comprava cinco segundos.** Com o intervalo, ele vira limite
/// de TAXA — e o teste prova as duas metades: recusa dentro da hora, aceita
/// depois dela.
#[test]
fn intervalo_minimo_transforma_o_bound_em_limite_de_taxa() {
    use dom_vault::constants::MIN_NAV_PUBLISH_INTERVAL;

    let mut env = Env::sem_nav_publicado();
    let oracle = env.oracle.insecure_clone();

    let t0 = env.agora();
    assert_ok(env.publish_nav_raw(&oracle, 1_100_000, t0), "primeira");

    // Um segundo depois: dentro do bound de 15%, mas cedo demais.
    env.avancar_sem_oraculo(1);
    let t1 = env.agora();
    assert_dom_error(
        env.publish_nav_raw(&oracle, 1_150_000, t1),
        DomError::NavPublicacaoMuitoCedo,
        "segunda publicacao em um segundo",
    );

    // Um segundo antes de completar a hora: ainda não.
    env.avancar_sem_oraculo(MIN_NAV_PUBLISH_INTERVAL - 2);
    let t2 = env.agora();
    assert_dom_error(
        env.publish_nav_raw(&oracle, 1_150_000, t2),
        DomError::NavPublicacaoMuitoCedo,
        "um segundo antes da hora",
    );

    // A hora exata passa — limite inclusivo, como os outros do cofre.
    env.avancar_sem_oraculo(1);
    let t3 = env.agora();
    assert_ok(
        env.publish_nav_raw(&oracle, 1_150_000, t3),
        "a hora exata passa",
    );
    assert_eq!(env.vault().nav, 1_150_000);
}

/// **A válvula da mesa NÃO tem intervalo mínimo, e é de propósito.**
///
/// Quem atravessa por proposta, com dois votos, é caminho de crise. Pôr carência
/// nele seria travar o cofre justamente no dia em que ele precisa ser
/// destravado — e é o mesmo motivo de o `paused` não gatear a válvula.
#[test]
fn a_valvula_da_mesa_nao_espera_intervalo_nem_pausa() {
    let mut env = Env::sem_nav_publicado();
    let oracle = env.oracle.insecure_clone();
    let authority = env.authority.insecure_clone();

    let t0 = env.agora();
    assert_ok(
        env.publish_nav_raw(&oracle, 1_100_000, t0),
        "oraculo publica",
    );

    env.pause();
    env.avancar_sem_oraculo(1);
    let t1 = env.agora();

    // O oráculo esbarra na pausa antes mesmo do intervalo.
    assert_dom_error(
        env.publish_nav_raw(&oracle, 1_100_001, t1),
        DomError::Paused,
        "pausado, o caminho de rotina fecha",
    );

    // A mesa atravessa os dois: sem intervalo, sem gate de pausa.
    assert_ok(
        env.publish_nav_raw(&authority, 700_000, t1),
        "a valvula da mesa atravessa pausa e intervalo",
    );
    assert_eq!(env.vault().nav, 700_000);
}

/// Fora da matriz, mesma família: NAV carimbado no futuro antecipa
/// elegibilidade de D+30 e virada de ciclo, que andam pelo relógio da rede.
#[test]
fn publish_nav_com_timestamp_no_futuro_rejeita() {
    let mut env = Env::sem_nav_publicado();
    let oracle = env.oracle.insecure_clone();
    let agora = env.agora();

    assert_dom_error(
        env.publish_nav_raw(&oracle, 1_100_000, agora + 3_600),
        DomError::NavTimestampInFuture,
        "NAV pos-datado",
    );
}

/// NAV zero dividiria cota por zero em todo o resto da cadeia.
#[test]
fn publish_nav_zero_rejeita() {
    let mut env = Env::sem_nav_publicado();
    let oracle = env.oracle.insecure_clone();
    let agora = env.agora();

    assert_dom_error(
        env.publish_nav_raw(&oracle, 0, agora),
        DomError::InvalidNav,
        "NAV zero",
    );
}

// ---------------------------------------------------------------------------
// T37 — publicação válida emite evento completo
// ---------------------------------------------------------------------------

/// NAV, hash de atestação e timestamp saem juntos no evento. O hash é o que
/// liga o número publicado à apuração que o produziu — NAV sem atestação é
/// opinião, e a conciliação de F3 depende desse par.
#[test]
fn t37_publish_nav_valido_emite_evento() {
    let mut env = Env::sem_nav_publicado();
    let oracle = env.oracle.insecure_clone();
    let t = env.agora();

    // 1,137000 sobre a genese de 1,000000 e' +13,7%: dentro do bound de 15%, que
    // e' o caminho do oraculo. O valor antigo (1,337000) era +33,7% e hoje so'
    // passa pela valvula da mesa — ver `dv11_valvula_da_mesa_publica_fora_do_bound`.
    let meta = assert_ok(env.publish_nav_raw(&oracle, 1_137_000, t), "T37 publicacao");
    assert!(tem_evento::<NavPublished>(&meta), "T37 evento NavPublished");

    let vault = env.vault();
    assert_eq!(vault.nav, 1_137_000);
    assert_eq!(vault.nav_ts, t, "timestamp gravado no estado");
    // DV12: a atestacao ficou no ESTADO, nao so' no evento.
    assert_eq!(
        vault.nav_attestation, [7u8; 32],
        "atestacao gravada no estado"
    );
}
