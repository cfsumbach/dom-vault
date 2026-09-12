//! **O custo em compute unit do transfer hook — medido, não presumido.**
//!
//! Existe por causa da `D-F2-19`. O `opt-level = "z"` corta ~17% do binário e
//! troca velocidade por tamanho — e velocidade, aqui, é **compute unit**. O hook
//! roda em **toda transferência de cota**: hook que estoura CU faz a cota parar
//! de transferir, e isso custa muito mais que os 0,7 SOL de aluguel economizados.
//!
//! A mesa recusou o `opt-z` na cerimônia com uma razão nomeada:
//!
//! > *"O CI passou verde com o `opt-z`. **E isso não basta:** os testes não
//! > imprimem CU, então 'passou' não é 'sei quanta margem sobrou'."*
//!
//! E fixou a condição para reconsiderar: **CU do hook medido nos dois perfis,
//! com a margem dita em número** — "consome X de Y, sobra Z%".
//!
//! Este arquivo é a resposta a essa condição. Ele **imprime** o consumo, e o
//! `--nocapture` é obrigatório para ler:
//!
//! ```bash
//! cargo test -p dom_vault --test custo_do_hook -- --nocapture
//! ```
//!
//! # O que o número significa, e o que ele NÃO significa
//!
//! O que se mede aqui é a **transação inteira**: o `transfer_checked` do
//! Token-2022 mais o CPI para o hook. Separar as duas metades exigiria
//! instrumentar o Token-2022, que não é nosso. **A soma é a métrica honesta**,
//! porque é ela que tem de caber no orçamento — o cotista não paga pelo hook,
//! paga pela transferência.
//!
//! O teto de referência é **200.000 CU**, o padrão por transação quando ninguém
//! pede `ComputeBudget`. É o pior caso realista: carteira comum, sem orçamento
//! ajustado. Quem monta a transação **pode** pedir até 1.400.000, e o cliente
//! deste repo já pede no `execute` — mas assumir isso seria medir contra o
//! ambiente amigável em vez do hostil.

#![allow(clippy::result_large_err)]

mod common;

use {common::*, solana_signer::Signer};

/// Teto padrão por transação, sem `ComputeBudget` — o pior caso realista.
const TETO_PADRAO: u64 = 200_000;

/// Margem mínima que a mesa aceita. Abaixo disto o perfil não entra.
///
/// 30% não é número redondo por acaso: é o que sobra para o Token-2022 ganhar
/// extensão nova, para uma conta a mais no `ExtraAccountMetaList`, e para o
/// runtime reprecificar syscall — três coisas que já aconteceram nesta cadeia e
/// nenhuma delas avisa antes.
const MARGEM_MINIMA_PCT: u64 = 30;

#[test]
fn custo_da_transferencia_com_hook() {
    let mut env = Env::new();
    let a = env.cotista(10_000 * UNIT);
    let b = env.cotista(10_000 * UNIT);

    let meta = assert_ok(
        env.transfer(&a, &b.dom, &b.wallet.pubkey(), 100 * UNIT),
        "transferencia de cota com hook",
    );

    let usado = meta.compute_units_consumed;
    let sobra = TETO_PADRAO.saturating_sub(usado);
    let pct = sobra * 100 / TETO_PADRAO;

    println!();
    println!("  ============================================================");
    println!("   CUSTO DA TRANSFERENCIA DE COTA (transfer_checked + hook)");
    println!("  ============================================================");
    println!("   consome ....... {usado} CU");
    println!("   de ............ {TETO_PADRAO} CU  (padrao, sem ComputeBudget)");
    println!("   sobra ......... {sobra} CU  ({pct}%)");
    println!("  ============================================================");
    println!();

    assert!(
        pct >= MARGEM_MINIMA_PCT,
        "margem de {pct}% abaixo do minimo de {MARGEM_MINIMA_PCT}% que a mesa \
         aceita (consumiu {usado} de {TETO_PADRAO}). Perfil de compilacao que \
         chegue aqui NAO entra: hook que estoura CU faz a cota parar de \
         transferir, e isso custa mais que qualquer economia de aluguel."
    );
}

/// O caminho mais caro que passa pelo hook: transferência **com o cap ligado**.
///
/// Com `cap_enforced`, o hook faz trabalho a mais — lê o supply do mint e compara
/// contra o teto por carteira. É este o número que decide, não o do caminho
/// barato: medir só o caso fácil é a mesma falha de medir só o CI verde.
#[test]
fn custo_da_transferencia_com_o_cap_ligado() {
    let mut env = Env::new();
    // SEIS cotistas, e nao dois. Com dois, cada um tem 50% do supply e o cap de
    // 25% recusa a transferencia antes de o hook chegar ao trabalho caro — o
    // teste mediria a recusa, nao o caminho. Com seis, cada um fica em ~16,7% e
    // sobra folga para o destino receber sem estourar.
    let a = env.cotista(10_000 * UNIT);
    let b = env.cotista(10_000 * UNIT);
    for _ in 0..4 {
        env.cotista(10_000 * UNIT);
    }
    env.enable_cap();

    let meta = assert_ok(
        env.transfer(&a, &b.dom, &b.wallet.pubkey(), UNIT),
        "transferencia com cap ligado",
    );

    let usado = meta.compute_units_consumed;
    let sobra = TETO_PADRAO.saturating_sub(usado);
    let pct = sobra * 100 / TETO_PADRAO;

    println!();
    println!("  ============================================================");
    println!("   CUSTO COM O CAP LIGADO — o caminho mais caro do hook");
    println!("  ============================================================");
    println!("   consome ....... {usado} CU");
    println!("   de ............ {TETO_PADRAO} CU");
    println!("   sobra ......... {sobra} CU  ({pct}%)");
    println!("  ============================================================");
    println!();

    assert!(
        pct >= MARGEM_MINIMA_PCT,
        "margem de {pct}% abaixo do minimo de {MARGEM_MINIMA_PCT}% (consumiu \
         {usado} de {TETO_PADRAO}) no caminho do cap ligado."
    );
}
