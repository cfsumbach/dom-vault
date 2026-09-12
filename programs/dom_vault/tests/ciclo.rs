//! **T56 — ciclo completo, ponta a ponta.**
//!
//! `deposit` → `publish_nav` → `deposit_especial` →
//! `solicitar_resgate_capital` → `efetivar_resgate_capital` → `redeem_fee_share`,
//! na ordem, num ambiente só.
//!
//! **Isto não fecha o T56.** A matriz exige TXs em devnet como evidência; o que
//! está aqui é a mesma sequência em LiteSVM. O valor é duplo: prova que a cadeia
//! fecha ponta a ponta antes de qualquer deploy, e é o roteiro exato que o
//! script de devnet vai executar — se mudar aqui, muda lá.

#![allow(clippy::result_large_err)]

mod common;

use {common::*, dom_vault::constants::NAV_SCALE, solana_signer::Signer};

#[test]
fn t56_ciclo_completo() {
    let mut env = Env::new();

    // -----------------------------------------------------------------------
    // 1. deposit — dois cotistas entram na gênese, 1:1.
    // -----------------------------------------------------------------------
    // 10.000 cada, para a saida de Ana caber no minimo de resgate de capital
    // (5.000 cotas). Os NAVs e o delta nao dependem do volume.
    let ana = env.cotista(10_000 * UNIT);
    let bruno = env.cotista(10_000 * UNIT);

    assert_eq!(env.supply_dom(), 20_000 * UNIT);
    assert_eq!(env.caixa(), 20_000 * UNIT);
    assert_eq!(env.vault().nav, NAV_SCALE);

    // -----------------------------------------------------------------------
    // 2. publish_nav — a mesa fecha o trimestre com +20%.
    // -----------------------------------------------------------------------
    env.publish_nav(1_200_000);
    assert_eq!(env.vault().nav, 1_200_000);
    assert_eq!(
        env.vault().lucro_sacavel_restante,
        0,
        "NAV subindo sozinho nao abre janela: marcacao nao e' lucro realizado"
    );

    // -----------------------------------------------------------------------
    // 3. deposit_especial — o lucro REALIZADO do ciclo entra e se reparte
    //    60/20/20/... 4.000 USDC de P sobre 20.000 cotas ao NAV 1,20:
    //      aos socios  = 2.400 (800 cada)
    //      aos cotistas = 1.600 -> NAV = (24.000 + 1.600) / 20.000 = 1,280000
    // -----------------------------------------------------------------------
    let patrimonio_antes = env.supply_dom() as u128 * env.vault().nav as u128 / UNIT as u128;
    env.deposit_especial(4_000 * UNIT);

    let vault = env.vault();
    assert_eq!(vault.nav, 1_280_000, "NAV pos-distribuicao");
    assert_eq!(vault.delta_lucro_por_cota, 80_000, "delta congelado");
    assert_eq!(
        vault.lucro_sacavel_restante,
        1_600 * UNIT,
        "janela aberta com 40% de P"
    );
    for socio in env.socios.iter() {
        assert!(
            env.ledger(&socio.wallet.pubkey()).shares > 0,
            "cada socio tem fee_share no livro"
        );
    }
    let patrimonio_depois = env.supply_dom() as u128 * vault.nav as u128 / UNIT as u128;
    assert!(
        (patrimonio_antes + 4_000 * UNIT as u128).abs_diff(patrimonio_depois) < UNIT as u128 / 100,
        "a taxa dilui, nao tira dinheiro do fundo — o patrimonio cresce o P inteiro"
    );

    // -----------------------------------------------------------------------
    // 4. solicitar_resgate_capital — Ana pede saída. Transferência para o
    //    escrow + pedido, na mesma transação (D17).
    // -----------------------------------------------------------------------
    let cotas_ana = env.saldo_dom(&ana);
    let id = env.solicitar_resgate(&ana, cotas_ana);

    assert_eq!(env.saldo_dom(&ana), 0, "cotas foram para o escrow");
    assert_eq!(env.saldo_escrow(), cotas_ana);
    assert_eq!(env.vault().cotas_travadas_resgate, cotas_ana);
    let teto = env.pedido(id).nav_na_solicitacao;
    assert_eq!(teto, 1_280_000, "o NAV do pedido e' TETO, nao preco");

    // -----------------------------------------------------------------------
    // 5. o período — o NAV oscila, a catraca desce, e a efetivação paga pelo
    //    PIOR, não pelo NAV do dia. É a diferença que o painel tem de mostrar.
    // -----------------------------------------------------------------------
    env.publish_nav(1_150_000); // cai
    env.carimbar_nav(id);
    env.publish_nav(1_300_000); // volta a subir
    env.carimbar_nav(id);

    let pior = env.pedido(id).pior_nav_visto;
    assert_eq!(pior, 1_150_000, "a catraca guardou o fundo do poco");
    assert!(
        pior < env.vault().nav,
        "o NAV de hoje ({}) e' maior que o que se vai pagar ({pior})",
        env.vault().nav
    );

    let pagadora = env.caixa_da_autoridade(50_000 * UNIT);
    env.update_endereco_resgate(pagadora);

    let usdc_antes = env.saldo_usdc(&ana);
    env.efetivar_resgate(id, pior, ana.usdc);
    let recebido = env.saldo_usdc(&ana) - usdc_antes;

    assert_eq!(recebido, cotas_ana * pior / NAV_SCALE);
    assert_eq!(env.saldo_escrow(), 0, "a cota foi queimada");
    assert_eq!(env.vault().cotas_travadas_resgate, 0);
    assert_eq!(env.pedido(id).estado, 1);

    // Ana saiu com lucro apesar de ter pago o pior NAV do período: entrou com
    // 1.000 e a distribuição a levou a 1,28 antes da queda.
    assert!(recebido > 10_000 * UNIT, "saiu com lucro: {recebido}");
    assert!(
        recebido < cotas_ana * teto / NAV_SCALE,
        "e saiu com MENOS do que o teto do pedido — e' esse o custo do prazo"
    );

    // 6. redeem_fee_share — um sócio saca a taxa contra caixa livre.
    // -----------------------------------------------------------------------
    let socio = Holder {
        wallet: env.socios[0].wallet.insecure_clone(),
        dom: env.socios[0].dom,
        usdc: env.socios[0].usdc,
    };
    let fee = env.ledger(&socio.wallet.pubkey()).shares;
    let usdc_socio_antes = env.saldo_usdc(&socio);

    env.redeem_fee_share(&socio, fee);

    assert_eq!(
        env.ledger(&socio.wallet.pubkey()).shares,
        0,
        "livro baixado"
    );
    assert_eq!(env.saldo_dom(&socio), 0, "cota queimada");
    assert!(
        env.saldo_usdc(&socio) > usdc_socio_antes,
        "socio recebeu USDC"
    );

    // -----------------------------------------------------------------------
    // Fecho: quem sobrou é Bruno com a cota dele e dois sócios com fee_share.
    // -----------------------------------------------------------------------
    let sobra: u64 = std::iter::once(env.saldo_dom(&bruno))
        .chain(env.socios.iter().map(|s| saldo(&env.svm, &s.dom)))
        .sum();
    assert_eq!(env.supply_dom(), sobra, "supply e' a soma do que sobrou");
    assert!(env.caixa() > 0, "caixa ainda honra o que resta");
}
