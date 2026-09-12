//! **O custo em compute unit do `deposit` — medido, não presumido.**
//!
//! A Camada 2 pediu o piso: quer montar `swap da Jupiter + deposit` na mesma
//! transação e precisa separar *"a rota é cara"* de *"o `deposit` é caro"*.
//! Sem número, a conversa vira palpite — e foi por causa dessa família de
//! palpite que a `D-F2-19` passou a exigir CU medido, não presumido.
//!
//! Mesma disciplina do `custo_do_hook.rs`: o teto de referência é **200.000 CU**,
//! o padrão por transação quando ninguém pede `ComputeBudget`.
//!
//! Duas medições, porque o caminho tem dois preços:
//!  . com `cap_enforced` **desligado** — é o estado da mainnet hoje;
//!  . com `cap_enforced` **ligado** — dois `reload()` a mais, e é para onde o
//!    cofre vai quando a base de cotistas crescer.

#![allow(clippy::result_large_err)]

mod common;

use common::*;

/// Teto padrão por transação, sem `ComputeBudget`.
const TETO_PADRAO: u64 = 200_000;

/// **O `.so` que estas medições descrevem — o do Upgrade F, vivo em mainnet.**
///
/// O número de CU só vale alguma coisa se estiver amarrado a um binário. Sem
/// esta trava, alguém constrói para o Upgrade G, os CU mudam, e a frase "medido
/// contra o binário de produção" — que está escrita no `docs/evidencias` e num
/// recado à Camada 2 — vira mentira **sem ninguém notar**.
///
/// Quando o G subir, este número muda **de propósito**, no mesmo commit que
/// atualizar o que o painel publica.
/// **Os binarios que este arquivo sabe medir, e qual esta' na rede.**
///
/// O numero de CU so' vale se estiver amarrado a um binario. Sem isto, alguem
/// constroi para o upgrade seguinte, os CU mudam, e a frase "medido contra o
/// binario de producao" — escrita numa evidencia commitada e num recado a
/// Camada 2 — vira mentira **sem ninguem notar**.
///
/// Nao basta travar num sha so': durante a construcao de um upgrade o binario
/// do disco E' o novo, e um teste que recusa os dois vira teste desligado.
/// Entao ele conhece os dois e **diz qual esta' medindo**.
const BINARIOS: &[(&str, &str)] = &[
    (
        "a0ea6c16717154f287a8a73146b7e247ae3272b313685b028569f4f532429833",
        "Upgrade F — VIVO EM MAINNET",
    ),
    (
        "9215d35797dc2c6f72542c64c9e9a2c553bf54bbf67f6e92277f8e48d7416ae5",
        "Upgrade G — construido, NAO instalado",
    ),
];

fn sha_do_elf() -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(program_bytes());
    format!("{:x}", h.finalize())
}

/// Devolve o rotulo do binario. **Nao falha quando nao reconhece** — rotula.
///
/// A primeira versao disto entrava em panico com sha desconhecido, e estava
/// errada: durante a construcao de um upgrade o binario muda a cada edicao da
/// fonte, e um teste que reprova a cada `cargo build-sbf` e' um teste que a
/// proxima pessoa deleta. Guarda que grita sempre acaba removida — e' a mesma
/// licao do portao de autoria que vivia vermelho.
///
/// O que precisa ser impossivel nao e' medir um binario novo: e' **confundir**
/// uma medicao de banca com uma de producao. Entao o sha vai no cabecalho do
/// relatorio, sempre, e binario desconhecido sai dito com todas as letras.
fn binario_medido() -> String {
    let atual = sha_do_elf();
    match BINARIOS.iter().find(|(sha, _)| *sha == atual) {
        Some((_, rotulo)) => rotulo.to_string(),
        None => format!("⚠ BINARIO EM CONSTRUCAO, nao publicado — {}", &atual[..16]),
    }
}

/// Quantas carteiras diferentes medir. **Nao e' zelo: o numero VARIA com o
/// endereco.**
///
/// A primeira versao deste arquivo imprimia um valor so', e eu o publiquei como
/// medicao. Rodando duas vezes: 28.041 e 26.541, mesmo binario conferido por
/// sha. A causa e' o `require_whitelisted`, que deriva a PDA dentro do handler
/// com `find_program_address` — e a moagem do bump custa ~1.500 CU por volta,
/// com o numero de voltas dependendo da CARTEIRA.
///
/// Uma amostra so' e' sorte apresentada como evidencia.
const AMOSTRAS: usize = 8;

struct Faixa {
    minimo: u64,
    maximo: u64,
}

fn relata_faixa(titulo: &str, f: &Faixa) {
    let sobra = TETO_PADRAO.saturating_sub(f.maximo);
    let pct = sobra * 100 / TETO_PADRAO;
    println!();
    println!("  ============================================================");
    println!("   {titulo}");
    println!("  ============================================================");
    println!("   {AMOSTRAS} carteiras distintas");
    println!("   consome ....... {} a {} CU", f.minimo, f.maximo);
    println!("   variacao ...... {} CU  (moagem do bump da whitelist)", f.maximo - f.minimo);
    println!("   de ............ {TETO_PADRAO} CU  (padrao, sem ComputeBudget)");
    println!("   sobra (pior) .. {sobra} CU  ({pct}%)");
    println!("  ============================================================");
    println!();
}

#[test]
fn custo_do_deposit_com_cap_desligado() {
    let binario = binario_medido();
    let mut medidas = Vec::new();
    for _ in 0..AMOSTRAS {
        let mut env = Env::new();
        assert!(!env.vault().cap_enforced, "cap comeca desligado");
        let cotista = env.carteira(1_000 * UNIT);
        medidas.push(env.deposit(&cotista, 1_000 * UNIT).compute_units_consumed);
    }
    let f = Faixa {
        minimo: *medidas.iter().min().unwrap(),
        maximo: *medidas.iter().max().unwrap(),
    };
    relata_faixa(&format!("CUSTO DO deposit — cap DESLIGADO · {binario}"), &f);
    assert!(
        f.maximo < TETO_PADRAO / 2,
        "o pior caso tem de caber com folga larga no teto padrao"
    );
}

#[test]
fn custo_do_deposit_com_cap_ligado() {
    let binario = binario_medido();
    let mut medidas = Vec::new();
    for _ in 0..AMOSTRAS {
        let mut env = Env::new();
        // O cap precisa de supply para ser satisfeito: sem cota emitida, o
        // primeiro depositante fica com 100% e a trava recusa.
        let _a = env.cotista(10_000 * UNIT);
        let _b = env.cotista(10_000 * UNIT);
        env.enable_cap();
        let cotista = env.carteira(1_000 * UNIT);
        medidas.push(env.deposit(&cotista, 1_000 * UNIT).compute_units_consumed);
    }
    let f = Faixa {
        minimo: *medidas.iter().min().unwrap(),
        maximo: *medidas.iter().max().unwrap(),
    };
    relata_faixa(&format!("CUSTO DO deposit — cap LIGADO · {binario}"), &f);
    assert!(f.maximo < TETO_PADRAO / 2, "idem");
}
