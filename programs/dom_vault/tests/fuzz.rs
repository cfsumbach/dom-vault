//! Grupo H da matriz — **aritmética e arredondamento** (T50–T53).
//!
//! Fuzz aqui não é gerar entrada aleatória e ver se explode: é rodar a cadeia
//! inteira com valores quebrados e conferir **invariantes** depois de cada
//! passo. Um teste que só olha "não deu erro" passaria com o cofre vazando
//! dinheiro para o lugar errado.
//!
//! As três invariantes que valem para qualquer sequência:
//!
//! 1. **Caixa fecha.** `treasury == Σ depositado − Σ pago`. Todo USDC que entrou
//!    ou está no caixa ou saiu para um cotista; não existe terceiro destino.
//! 2. **Escrow bate com o livro.** `vault.cotas_travadas_resgate == escrow.amount`. É a
//!    nossa contabilidade contra a verdade do token.
//! 3. **Supply é a soma dos saldos.** Nenhuma cota nasce fora de `deposit` e
//!    `accrue_performance`, nenhuma some fora de `process_redemptions` e
//!    `redeem_fee_share`.
//!
//! Semente fixa: teste vermelho é reproduzível.

#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::constants::{MIN_RESGATE_CAPITAL_USDC, NAV_SCALE},
    solana_signer::Signer,
};

/// xorshift64*. Determinístico e sem dependência nova — o valor aqui é
/// reprodutibilidade, não qualidade estatística.
struct Rng(u64);

impl Rng {
    fn proximo(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Inteiro em `[min, max]`, inclusive.
    fn entre(&mut self, min: u64, max: u64) -> u64 {
        min + self.proximo() % (max - min + 1)
    }
}

/// Confere as três invariantes. `contas` são todas as contas de DOM
/// conhecidas — cotistas, sócios e escrow.
fn conferir_invariantes(env: &Env, contas: &[Pubkey], depositado: u64, pago: u64, onde: &str) {
    // **O treasury não paga resgate.** O resgate de capital sai do
    // `endereco_resgate`, que a mesa abastece de fora do cofre — então o caixa
    // fecha com o que entrou, e o `pago` é conferido à parte. A invariante ficou
    // mais forte do que era com a fila D+30: antes o caixa subia e descia; agora
    // o cofre só recebe.
    assert_eq!(
        env.caixa(),
        depositado,
        "{onde}: caixa nao fecha (depositado {depositado}, pago {pago} veio de fora)"
    );

    let vault = env.vault();
    assert_eq!(
        vault.cotas_travadas_resgate,
        env.saldo_escrow(),
        "{onde}: livro de escrow diverge do saldo real"
    );

    let soma: u64 = contas.iter().map(|c| saldo(&env.svm, c)).sum();
    assert_eq!(
        soma,
        env.supply_dom(),
        "{onde}: supply nao e' a soma dos saldos"
    );
}

use anchor_lang::prelude::Pubkey;

// ---------------------------------------------------------------------------
// T50 — a cadeia inteira com valores quebrados
// ---------------------------------------------------------------------------

/// Depósitos, apurações, pedidos e processamentos em ordem sorteada, todos com
/// valores quebrados. As invariantes são conferidas depois de **cada** passo, e
/// nenhum cotista pode sair com mais USDC do que colocou enquanto o NAV nunca
/// subir acima do de entrada.
#[test]
fn t50_fuzz_da_cadeia_completa_conserva_valor() {
    let mut rng = Rng(0x5E4D_0B0F_2026_0817);
    let mut env = Env::new();

    let bolso = 100_000 * UNIT;
    let cotistas: Vec<Holder> = (0..3).map(|_| env.carteira(bolso)).collect();

    // Conta pagadora dos resgates, abastecida pela mesa. Sem ela a efetivação
    // recusa por saldo e o passo nunca roda — o `pago > 0` no fim é o piso de
    // cobertura que pega isso.
    let pagadora = env.caixa_da_autoridade(200_000 * UNIT);
    env.update_endereco_resgate(pagadora);

    let mut depositado: u64 = 0;
    let mut pago: u64 = 0;
    // Pedidos de resgate abertos: (id, indice do cotista).
    let mut abertos: Vec<(u64, usize)> = Vec::new();
    // USDC que cada cotista colocou, para o teste de não-criação de valor.
    let mut aportado = [0u64; 3];

    let contas: Vec<Pubkey> = cotistas
        .iter()
        .map(|c| c.dom)
        .chain(env.socios.iter().map(|s| s.dom))
        .chain(std::iter::once(escrow_dom_pda()))
        .collect();

    for rodada in 0..24 {
        match rng.entre(0, 3) {
            // Depósito com valor quebrado.
            0 => {
                let i = rng.entre(0, 2) as usize;
                let valor = rng.entre(6_000 * UNIT, 20_000 * UNIT);
                if saldo(&env.svm, &cotistas[i].usdc) >= valor {
                    env.deposit(&cotistas[i], valor);
                    depositado += valor;
                    aportado[i] += valor;
                }
            }
            // NAV novo, sempre <= 1,000000: assim "não criar valor" vira uma
            // desigualdade simples de conferir no fim.
            1 => {
                // NAV novo, sem distribuição nenhuma: marcação não paga taxa e
                // não minta cota. É a régua do D-F2-09 dentro do fuzz.
                let nav = rng.entre(NAV_SCALE / 2, NAV_SCALE);
                env.publish_nav(nav);
            }
            // Pedido de resgate de capital com valor quebrado.
            2 => {
                let i = rng.entre(0, 2) as usize;
                let saldo_dom = env.saldo_dom(&cotistas[i]);
                // Abaixo do mínimo o pedido é recusado, então o fuzz sorteia
                // dentro da faixa válida em vez de bater na trava toda rodada e
                // nunca exercer o mecanismo. `raw` porque outras travas
                // (whitelist, pausa) podem recusar legitimamente.
                if saldo_dom > MIN_RESGATE_CAPITAL_USDC {
                    let cotas = rng.entre(MIN_RESGATE_CAPITAL_USDC, saldo_dom);
                    if env.solicitar_resgate_raw(&cotistas[i], cotas).is_ok() {
                        abertos.push((env.vault().proximo_pedido_id - 1, i));
                    }
                }
            }
            // Efetiva um pedido aberto, pelo pior NAV carimbado.
            _ => {
                if !abertos.is_empty() {
                    let k = rng.entre(0, abertos.len() as u64 - 1) as usize;
                    let (id, dono) = abertos.remove(k);
                    env.carimbar_nav(id);
                    let pior = env.pedido(id).pior_nav_visto;
                    let antes = env.saldo_usdc(&cotistas[dono]);
                    env.efetivar_resgate(id, pior, cotistas[dono].usdc);
                    pago += env.saldo_usdc(&cotistas[dono]) - antes;
                }
            }
        }

        conferir_invariantes(&env, &contas, depositado, pago, &format!("rodada {rodada}"));
    }

    // Liquida o que sobrou aberto, para o estado final ser comparável.
    while let Some((id, dono)) = abertos.pop() {
        env.carimbar_nav(id);
        let pior = env.pedido(id).pior_nav_visto;
        let antes = env.saldo_usdc(&cotistas[dono]);
        env.efetivar_resgate(id, pior, cotistas[dono].usdc);
        pago += env.saldo_usdc(&cotistas[dono]) - antes;
    }
    conferir_invariantes(&env, &contas, depositado, pago, "final");

    // Nenhum cotista saiu com mais do que colocou: o NAV nunca passou de
    // 1,000000, então resgate nenhum pode devolver mais que o aporte.
    for (i, c) in cotistas.iter().enumerate() {
        // Saldo final = bolso - aportado + recebido.
        let recebido = env.saldo_usdc(c) + aportado[i] - bolso;
        assert!(
            recebido <= aportado[i],
            "cotista {i} recebeu {recebido} tendo aportado {}",
            aportado[i]
        );
    }
    assert!(
        depositado > 0 && pago > 0,
        "o fuzz precisa exercitar os dois lados"
    );
}

// ---------------------------------------------------------------------------
// T51 — NAV próximo de zero
// ---------------------------------------------------------------------------

/// Lição do `shares_base` do Drift: é perto de zero que a cota infla.
///
/// Com NAV em 0,000001 — o menor valor representável — um aporte de 200 USDC
/// vira 200 milhões de cotas. O que **não** pode acontecer é o caminho de volta
/// devolver mais do que entrou.
#[test]
fn t51_fuzz_com_nav_proximo_de_zero() {
    let mut rng = Rng(0x0000_0001_2026_0817);
    let mut env = Env::new();
    let pagadora = env.caixa_da_autoridade(100_000 * UNIT);
    env.update_endereco_resgate(pagadora);

    let mut cotistas: Vec<Holder> = Vec::new();

    for _ in 0..6 {
        let nav = rng.entre(1, 20); // 0,000001 a 0,000020
        env.publish_nav(nav);

        let cotista = env.carteira(30_000 * UNIT);
        let aporte = rng.entre(6_000 * UNIT, 20_000 * UNIT);
        env.deposit(&cotista, aporte);

        let cotas = env.saldo_dom(&cotista);
        assert!(cotas > 0, "aporte tem que comprar cota");

        // Ida e volta ao mesmo NAV nunca devolve mais do que entrou. Agora numa
        // efetivação só — não há mais cap de ciclo obrigando parcelamento.
        let id = env.solicitar_resgate(&cotista, cotas);
        let antes = env.saldo_usdc(&cotista);
        cotistas.push(cotista);
        let cotista = cotistas.last().unwrap();

        let pior = env.pedido(id).pior_nav_visto;
        let destino = cotista.usdc;
        env.efetivar_resgate(id, pior, destino);
        let devolvido = env.saldo_usdc_da_conta(destino) - antes;

        assert!(
            devolvido <= aporte,
            "NAV {nav}: aportou {aporte}, recebeu {devolvido}"
        );
        assert_eq!(
            env.vault().cotas_travadas_resgate,
            env.saldo_escrow(),
            "escrow bate mesmo com NAV minusculo"
        );
    }
}

// ---------------------------------------------------------------------------
// T52 — arredondamento sempre a favor do cofre
// ---------------------------------------------------------------------------

/// A metade do depósito já está em `deposit.rs`. Aqui é a do resgate: com NAV
/// quebrado, o valor pago é truncado, e o que sobra da divisão fica no cofre.
#[test]
fn t52_arredondamento_do_resgate_favorece_o_cofre() {
    let mut env = Env::new();
    let cotista = env.cotista(20_000 * UNIT);
    let pagadora = env.caixa_da_autoridade(50_000 * UNIT);
    env.update_endereco_resgate(pagadora);

    // NAV quebrado de propósito: 0,333333.
    env.publish_nav(333_333);

    // Quebrado E acima do mínimo de resgate: 5.000,000001 cotas.
    let cotas = 5_000 * UNIT + 1;
    let id = env.solicitar_resgate(&cotista, cotas);
    let pior = env.pedido(id).pior_nav_visto;
    assert_eq!(pior, 333_333);

    // 5.000.000.001 x 0,333333 = 1.666.664.999,666... -> trunca para baixo.
    let exato_truncado = (cotas as u128 * 333_333u128 / NAV_SCALE as u128) as u64;

    let antes = env.saldo_usdc(&cotista);
    env.efetivar_resgate(id, pior, cotista.usdc);
    let pago = env.saldo_usdc(&cotista) - antes;

    assert_eq!(pago, exato_truncado, "paga o truncado, nao o arredondado");
    // A fração perdida ficou no caixa, não no bolso do cotista.
    assert!(
        (pago as u128) * NAV_SCALE as u128 <= cotas as u128 * 333_333u128,
        "pagamento nunca excede o valor exato das cotas"
    );
}

// ---------------------------------------------------------------------------
// T53 — precisão de ponta a ponta
// ---------------------------------------------------------------------------

/// NAV `1_000_000` é 1,000000 exato, e a identidade tem que sobreviver à cadeia
/// inteira: `deposit` → `publish_nav` → `solicitar_resgate_capital` →
/// `efetivar_resgate_capital`. Sem perder nem uma unidade mínima.
#[test]
fn t53_precisao_preservada_na_cadeia() {
    let mut env = Env::new();
    let cotista = env.cotista(20_000 * UNIT);
    let outro = env.cotista(20_000 * UNIT);
    let pagadora = env.caixa_da_autoridade(50_000 * UNIT);
    env.update_endereco_resgate(pagadora);

    assert_eq!(env.saldo_dom(&cotista), 20_000 * UNIT, "1:1 na genese");

    // Republica o mesmo NAV: sem distribuição, nada muda.
    env.publish_nav(NAV_SCALE);
    assert_eq!(env.vault().nav, NAV_SCALE, "NAV intacto");
    assert_eq!(env.supply_dom(), 40_000 * UNIT, "supply intacto");

    let id = env.solicitar_resgate(&cotista, 20_000 * UNIT);
    assert_eq!(
        env.pedido(id).nav_na_solicitacao,
        NAV_SCALE,
        "o teto e' o NAV de 1,000000 exato"
    );

    let antes = env.saldo_usdc(&cotista);
    env.efetivar_resgate(id, NAV_SCALE, cotista.usdc);

    assert_eq!(
        env.saldo_usdc(&cotista) - antes,
        20_000 * UNIT,
        "ida e volta ao mesmo NAV devolve exatamente o que entrou"
    );
    assert_eq!(
        env.supply_dom(),
        20_000 * UNIT,
        "so restam as cotas do outro"
    );
    assert_eq!(
        env.caixa(),
        40_000 * UNIT,
        "o treasury NAO pagou: o resgate saiu do endereco de resgate"
    );
    assert_eq!(env.vault().cotas_travadas_resgate, 0);
    let _ = outro;
}

/// A mesma cadeia **com** lucro: a precisão tem que sobreviver à diluição da
/// taxa, que é onde a conta fica quebrada de verdade.
///
/// `P` escolhido quebrado de propósito (137 USDC, não 200): o objetivo é um NAV
/// pós-distribuição que não seja redondo, senão o truncamento das fatias de
/// pagamento nunca aparece e o teste passaria sem exercer o que promete.
///
///   supply 20.000 ao NAV 1,20 -> patrimônio 24.000
///   P = 1.370 -> sócios 822 / cotistas 548
///   NAV = (24.000 + 548) / 20.000 = 1,227400
///   o cotista tem metade das cotas: 12.000 de principal + 274 de lucro
#[test]
fn t53b_precisao_com_taxa_no_meio() {
    let mut env = Env::new();
    let cotista = env.cotista(10_000 * UNIT);
    // Um segundo cotista existe para que haja caixa também para ele.
    let _outro = env.cotista(10_000 * UNIT);
    let pagadora = env.caixa_da_autoridade(50_000 * UNIT);
    env.update_endereco_resgate(pagadora);

    env.publish_nav(1_200_000);
    env.deposit_especial(1_370 * UNIT);
    assert_eq!(env.vault().nav, 1_227_400, "NAV pos-distribuicao, quebrado");

    let nav = env.vault().nav;
    let cotas = env.saldo_dom(&cotista);
    let esperado = (cotas as u128 * nav as u128 / NAV_SCALE as u128) as u64;

    let id = env.solicitar_resgate(&cotista, cotas);
    assert_eq!(
        env.pedido(id).nav_na_solicitacao,
        nav,
        "o teto e' o NAV quebrado de agora"
    );

    let antes = env.saldo_usdc(&cotista);
    env.efetivar_resgate(id, nav, cotista.usdc);
    let recebido = env.saldo_usdc(&cotista) - antes;
    assert_eq!(recebido, esperado, "sem dust: a conta fecha na unidade");

    // -----------------------------------------------------------------------
    // Aqui aparece o único ponto da cadeia onde a precisão não é exata, e é
    // preciso ser explícito sobre ele.
    //
    // Cada fatia de pagamento parcial recalcula o congelado do remanescente por
    // `value × cotas_restantes / cotas`, **truncado** (D18.1). Truncar perde no
    // máximo uma unidade mínima por fatia — 0,000001 USDC — e perde **sempre
    // para o cofre**, nunca para o cotista (D9).
    //
    // Com a fila D+30 o pagamento saía em fatias e cada uma podia perder uma
    // unidade no truncamento — o teste tolerava esse dust. **A efetivação é uma
    // só**, então o dust desapareceu do desenho: a conta fecha exata, e o teste
    // afirma isso em vez de tolerar folga que não existe mais.
    // -----------------------------------------------------------------------
    assert_eq!(
        recebido, esperado,
        "uma efetivacao so: sem fatias, sem dust"
    );
    assert_eq!(
        esperado, 12_274_000_000,
        "10.000 cotas x 1,227400 — principal mais a fatia do cotista nos 40% de P"
    );
    assert!(
        recebido > 1_227 * UNIT,
        "recebido {recebido}: principal mais lucro liquido de taxa"
    );
    assert_eq!(
        env.vault().cotas_travadas_resgate,
        0,
        "escrow zerado, sem cota presa"
    );
}

// ---------------------------------------------------------------------------
// T66 — invariantes NEGATIVAS (auditoria F4-02, ordem 3)
//
// As três invariantes do topo deste arquivo são positivas: conferem que a
// contabilidade fecha. As cinco abaixo são negativas — cada uma nomeia um
// caminho que **não pode existir** e falha se ele existir.
//
// A diferença importa. "O supply é a soma dos saldos" continua verdadeiro se
// uma instrução nova mintar cota indevidamente e creditar alguém. "Supply só
// cresce em `deposit` ou `deposit_especial`" não.
//
// Harness nativo, semente fixa (o fuzz deste repo não é Trident nem cargo-fuzz;
// é a cadeia real em LiteSVM com valores sorteados e invariantes conferidas
// entre passos).
// ---------------------------------------------------------------------------

/// Qual instrução acabou de rodar. É o que permite às invariantes 1 e 2
/// distinguirem "mudou" de "mudou onde podia".
#[derive(Clone, Copy, PartialEq, Debug)]
enum Passo {
    Deposit,
    PublishNav,
    Distribuicao,
    SacarLucro,
    SolicitarResgate,
    EfetivarResgate,
    RedeemFeeShare,
    Transfer,
    Deploy,
    Nada,
}

impl Passo {
    /// Só estas duas mintam cota.
    fn pode_crescer_supply(self) -> bool {
        matches!(self, Passo::Deposit | Passo::Distribuicao)
    }
    /// Só estas **três** tiram USDC do caixa.
    ///
    /// **`EfetivarResgate` não está aqui, e é o ponto do desenho novo:** o
    /// resgate de capital se paga do `endereco_resgate`, que a mesa abastece de
    /// fora do cofre. O treasury não é fonte dele. Era `Process` que estava nesta
    /// lista, quando a fila D+30 se pagava do caixa.
    fn pode_sair_usdc(self) -> bool {
        matches!(
            self,
            Passo::RedeemFeeShare | Passo::Deploy | Passo::SacarLucro
        )
    }
}

struct Estado {
    supply: u64,
    caixa: u64,
    deployed_usdc: u64,
}

impl Estado {
    fn ler(env: &Env) -> Self {
        Estado {
            supply: env.supply_dom(),
            caixa: env.caixa(),
            deployed_usdc: env.vault().deployed_usdc,
        }
    }
}

/// As invariantes negativas, conferidas depois de **cada** passo.
///
/// Eram cinco na campanha adversarial (F4-02/F4-06). O bloco 3 da F2 acrescentou
/// a 2b — capital que sai tem que aparecer no contador — e reenunciou a 2 para
/// cobrir o `deploy_capital`, que e' a primeira saida de USDC para endereco de
/// **lista** e nao derivado do estado.
fn conferir_invariantes_negativas(
    env: &Env,
    antes: &Estado,
    passo: Passo,
    comuns: &[Pubkey],
    admitido: Option<Pubkey>,
    destino_do_deploy: Option<Pubkey>,
    rodada: usize,
) {
    let depois = Estado::ler(env);
    let vault = env.vault();

    // 1 — supply só cresce em `deposit` ou `deposit_especial`.
    if depois.supply > antes.supply {
        assert!(
            passo.pode_crescer_supply(),
            "rodada {rodada}: supply subiu de {} para {} em {passo:?} — só \
             deposit e deposit_especial mintam",
            antes.supply,
            depois.supply
        );
    }

    // 2 — USDC só sai da treasury em `process_redemptions`, `redeem_fee_share`
    // ou `deploy_capital` **para destino allowlisted**.
    //
    // A terceira entrou com a porta operacional da F2 e é diferente das outras
    // duas: as duas primeiras pagam quem tem direito — o dono do pedido na fila,
    // o sócio do livro —, para destinos **derivados** do próprio estado. O
    // `deploy_capital` manda dinheiro para endereço de **lista**, com valor
    // livre. Por isso a invariante não para em "foi o deploy": confere que o
    // destino estava mesmo na allowlist quando o caixa caiu.
    if depois.caixa < antes.caixa {
        assert!(
            passo.pode_sair_usdc(),
            "rodada {rodada}: caixa caiu de {} para {} em {passo:?} — só \
             process_redemptions, redeem_fee_share e deploy_capital pagam",
            antes.caixa,
            depois.caixa
        );
        if passo == Passo::Deploy {
            let destino = destino_do_deploy.expect("passo Deploy sem destino registrado");
            assert!(
                vault.deploy_allowlist.contains(&destino),
                "rodada {rodada}: deploy_capital tirou USDC para {destino}, \
                 que NAO esta na allowlist {:?}",
                vault.deploy_allowlist
            );
        }
    }

    // 2b — o contador de capital em campo acompanha a saída. Se o caixa cai por
    // `deploy_capital` e `deployed_usdc` não sobe, o backend de NAV perde o
    // rastro do dinheiro (a lição da DV12, agora do lado do capital).
    if passo == Passo::Deploy && depois.caixa < antes.caixa {
        let saiu = antes.caixa - depois.caixa;
        assert_eq!(
            depois.deployed_usdc,
            antes.deployed_usdc + saiu,
            "rodada {rodada}: saiu {saiu} do caixa e o contador de capital em \
             campo foi de {} para {}",
            antes.deployed_usdc,
            depois.deployed_usdc
        );
    }

    // 3 — o cap de 25% vale **na admissão**: mint E transferência.
    //
    // Enunciada como "nenhuma carteira comum passa de 25% ao fim de qualquer
    // instrução", esta invariante **falha** — e a falha é do desenho, não do
    // harness. O cap é checado no mint (`deposit`) e no destino da
    // transferência (hook). **Queima não recheca ninguém**, e queima reduz o
    // supply: quando um cotista resgata, o percentual de todos os outros sobe
    // sozinho. Contraexemplo determinístico em T67.
    //
    // O que o código de fato garante, e é o que se confere aqui: **logo depois
    // de uma admissão — mint do `deposit` ou destino de transferência — a conta
    // que recebeu está dentro dos 25%**. Os dois caminhos, porque são os dois
    // pontos onde o cap roda. Ver F4-02 e F4-06.
    if vault.cap_enforced && matches!(passo, Passo::Deposit | Passo::Transfer) {
        if let Some(conta) = admitido {
            let saldo_conta = saldo(&env.svm, &conta) as u128;
            let supply = depois.supply as u128;
            assert!(
                saldo_conta * 100 <= supply * 25,
                "rodada {rodada} ({passo:?}): admissao deixou a conta com \
                 {saldo_conta} de supply {supply} — passou de 25%"
            );
        }
    }
    let _ = comuns;

    // 4 — `redeem_fee_share` nunca come a reserva de 10% (D7/T43).
    //
    // Ao contrário do `process_redemptions`, que PODE consumi-la para honrar a
    // fila. A assimetria é o ponto da D7, e é o que esta invariante trava.
    if passo == Passo::RedeemFeeShare {
        let patrimonio = (depois.supply as u128) * (vault.nav as u128) / (NAV_SCALE as u128);
        let reserva = (patrimonio * vault.reserve_bps as u128) / 10_000;
        assert!(
            depois.caixa as u128 >= reserva,
            "rodada {rodada}: redeem_fee_share deixou o caixa em {} contra \
             reserva de {reserva} — comeu a reserva da fila",
            depois.caixa
        );
    }

    // 5 — o prometido em escrow nunca passa do que a conta de escrow tem.
    //
    // Mais forte que a invariante positiva do topo: aqui a soma vem das CONTAS
    // DE PEDIDO, uma a uma, não do contador do `Vault`. Se o contador e os
    // pedidos divergirem, esta pega e aquela não.
    let somado: u64 = (0..vault.proximo_pedido_id)
        .filter_map(|id| env.pedido_opt(id))
        .filter(|p| p.estado != 1) // 1 = pago; pago já queimou a cota
        .map(|p| p.cotas)
        .sum();
    assert!(
        somado <= env.saldo_escrow(),
        "rodada {rodada} ({passo:?}): os pedidos prometem {somado} cotas e o \
         escrow tem {}",
        env.saldo_escrow()
    );
    assert_eq!(
        somado, vault.cotas_travadas_resgate,
        "rodada {rodada} ({passo:?}): os pedidos somam {somado} e o Vault diz {}",
        vault.cotas_travadas_resgate
    );
}

/// T66 — as invariantes negativas contra sequências sorteadas.
///
/// Quatro sementes × 40 rodadas. O cap fica **ativo** desde o começo para que a
/// invariante 3 tenha o que conferir; depósitos recusados por cap são passo
/// legítimo e entram como `Nada`.
#[test]
fn t66_invariantes_negativas() {
    // Re-fuzz do bloco 4 da F2: o laço passou a exercer código novo — staleness,
    // bound, cadência, mínimo de resgate, teto por carteira e a porta
    // operacional. Mais sementes e mais rodadas porque o espaço de estado
    // cresceu junto: sorteio que só cobria o cofre fechado não cobre mais o
    // programa inteiro.
    //
    // Re-fuzz do pacote do rendimento (D-F2-09): entraram `deposit_especial` e
    // `sacar_lucro`, e com eles um estado que o sorteio antes não tinha — a
    // **janela aberta**, que fecha o `deposit` e a transferência de cota. Quatro
    // sementes novas, porque o laço agora atravessa dois regimes (janela aberta
    // e fechada) e os caminhos de recusa de um são caminho normal do outro.
    let sementes: [u64; 12] = [
        0x0000_0001_2026_0820,
        0x5E4D_0B0F_2026_0820,
        0xA5A5_5A5A_2026_0820,
        0x1234_5678_9ABC_DEF0,
        0xDEAD_BEEF_2026_0820,
        0x0F0F_F0F0_2026_0820,
        0x7777_1111_2026_0820,
        0xCAFE_D00D_2026_0820,
        0x0000_0001_2026_0823,
        0x5E4D_0B0F_2026_0823,
        0xB0CA_1DA0_2026_0823,
        0x9E57_2026_0823_0001,
    ];

    let mut sequencias = 0usize;
    let mut passos_efetivos = 0usize;
    // Cobertura por tipo de passo. Re-fuzz sem cobertura medida e' numero, nao
    // evidencia: um laco que sorteia 480 vezes e nunca consegue executar um
    // `deploy_capital` passaria verde sem ter exercido nada do bloco novo.
    let mut por_passo: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for semente in sementes {
        let mut rng = Rng(semente);
        let mut env = Env::new();

        // Bolso folgado: com 5.000 os cotistas ficavam sem USDC depois de meia
        // duzia de aportes e o passo `Deposit` sumia do sorteio — o piso de
        // cobertura la' embaixo pegou isso. O cap continua sendo a trava que
        // recusa deposito, que e' o que interessa exercer.
        // **Cinco** carteiras, e só quatro depositam na largada.
        //
        // Com quatro cotistas em 25% exatos, todo aporte novo empurraria o
        // depositante para cima de 25% e o cap recusaria — o passo `Deposit`
        // virava quase só recusa, e a invariante 3 (cap na admissão) ficava sem
        // amostra. A quinta carteira entra em 0% e tem espaço para ser admitida,
        // que é justamente o caminho que a invariante 3 confere.
        //
        // O primeiro depósito de cada um acontece antes de ligar o cap: com
        // supply zero o primeiro depositante teria 100% e o cap rejeitaria tudo
        // (D3).
        let cotistas: Vec<Holder> = (0..5).map(|_| env.carteira(200_000 * UNIT)).collect();
        for c in cotistas.iter().take(4) {
            env.deposit(c, 20_000 * UNIT);
        }
        env.enable_cap();

        let comuns: Vec<Pubkey> = cotistas.iter().map(|c| c.dom).collect();

        // Carteira operacional registrada na allowlist, para o passo de
        // `deploy_capital` existir. A segunda vaga fica livre de propósito: um
        // destino nao registrado tem que continuar sendo recusado no meio do
        // sorteio, e nao so' no teste deterministico.
        let ops = env.carteira(0);
        env.update_deploy_allowlist(0, ops.wallet.pubkey());
        let estranho = env.carteira(0);

        // Conta pagadora dos resgates, abastecida com folga. É a mesa
        // abastecendo de fora do cofre — o passo de efetivação existe por causa
        // dela, e quando o saldo acabar a recusa é legítima e vira `Nada`.
        let pagadora = env.caixa_da_autoridade(200_000 * UNIT);
        env.update_endereco_resgate(pagadora);

        // Pedidos de resgate abertos: (id, índice do cotista).
        let mut abertos: Vec<(u64, usize)> = Vec::new();

        for rodada in 0..60 {
            let antes = Estado::ler(&env);

            let mut admitido: Option<Pubkey> = None;
            let mut destino_do_deploy: Option<Pubkey> = None;
            let passo = match rng.entre(0, 8) {
                0 => {
                    let i = rng.entre(0, 4) as usize;
                    let valor = rng.entre(2_000 * UNIT, 8_000 * UNIT);
                    if env.saldo_usdc(&cotistas[i]) >= valor
                        && env.deposit_raw(&cotistas[i], valor).is_ok()
                    {
                        admitido = Some(cotistas[i].dom);
                        Passo::Deposit
                    } else {
                        Passo::Nada
                    }
                }
                1 => {
                    let nav = rng.entre(NAV_SCALE / 2, NAV_SCALE * 2);
                    env.publish_nav(nav);
                    Passo::PublishNav
                }
                2 => {
                    // DV10: dentro da quinzena a distribuicao e' recusada.
                    // Recusa e' passo legitimo do sorteio, nao falha do fuzz —
                    // entra como `Nada`, igual ao deposito barrado pelo cap.
                    let lucro = rng.entre(10 * UNIT, 100 * UNIT);
                    let origem = env.caixa_da_autoridade(lucro);
                    let authority = env.authority.insecure_clone();
                    if env.deposit_especial_raw(&authority, origem, lucro).is_ok() {
                        Passo::Distribuicao
                    } else {
                        Passo::Nada
                    }
                }
                3 => {
                    let i = rng.entre(0, 3) as usize;
                    let saldo_dom = env.saldo_dom(&cotistas[i]);
                    if saldo_dom > MIN_RESGATE_CAPITAL_USDC
                        && env
                            .solicitar_resgate_raw(
                                &cotistas[i],
                                rng.entre(MIN_RESGATE_CAPITAL_USDC, saldo_dom),
                            )
                            .is_ok()
                    {
                        abertos.push((env.vault().proximo_pedido_id - 1, i));
                        Passo::SolicitarResgate
                    } else {
                        Passo::Nada
                    }
                }
                4 => {
                    // Efetiva um pedido aberto pelo pior NAV carimbado. Carimbar
                    // antes é o que o backend faria a cada publicação.
                    if abertos.is_empty() {
                        Passo::Nada
                    } else {
                        let k = rng.entre(0, abertos.len() as u64 - 1) as usize;
                        let (id, dono) = abertos.remove(k);
                        env.carimbar_nav(id);
                        let pior = env.pedido(id).pior_nav_visto;
                        let destino = cotistas[dono].usdc;
                        if env
                            .efetivar_resgate_raw(
                                &env.authority.insecure_clone(),
                                id,
                                pior,
                                destino,
                            )
                            .is_ok()
                        {
                            Passo::EfetivarResgate
                        } else {
                            // Sem saldo no endereço de resgate: recusa legítima.
                            abertos.push((id, dono));
                            Passo::Nada
                        }
                    }
                }
                5 => {
                    // Transferência entre cotistas — o OUTRO ponto onde o cap
                    // roda (no hook, contra o destino). Sem este passo, a
                    // invariante 3 cobria só a perna do mint.
                    let de = rng.entre(0, 3) as usize;
                    let para = (de + 1 + rng.entre(0, 2) as usize) % 4;
                    let saldo_de = env.saldo_dom(&cotistas[de]);
                    if saldo_de > UNIT {
                        let quanto = rng.entre(UNIT, saldo_de);
                        let destino_dom = cotistas[para].dom;
                        let destino_dono = cotistas[para].wallet.pubkey();
                        if env
                            .transfer(&cotistas[de], &destino_dom, &destino_dono, quanto)
                            .is_ok()
                        {
                            admitido = Some(destino_dom);
                            Passo::Transfer
                        } else {
                            Passo::Nada
                        }
                    } else {
                        Passo::Nada
                    }
                }
                6 => {
                    // Saque de lucro na janela (D-F2-09). Fora da janela, ou
                    // pela segunda vez na mesma, e' recusado — e recusa e' passo
                    // legitimo do sorteio. O que o fuzz cobra aqui e' a
                    // invariante 2: o USDC que sai por este caminho e' saida
                    // legitima do caixa, e nenhuma outra o e'.
                    let i = rng.entre(0, 3) as usize;
                    if env.sacar_lucro_raw(&cotistas[i]).is_ok() {
                        Passo::SacarLucro
                    } else {
                        Passo::Nada
                    }
                }
                7 => {
                    // Porta operacional: metade das vezes tenta mandar para a
                    // carteira registrada, metade para uma que NAO esta na
                    // lista. A segunda tem que ser sempre recusada — e se algum
                    // dia passar, a invariante 2 pega o caixa caindo para
                    // destino fora da allowlist.
                    let legitimo = rng.entre(0, 1) == 0;
                    let (conta, dono) = if legitimo {
                        (ops.usdc, ops.wallet.pubkey())
                    } else {
                        (estranho.usdc, estranho.wallet.pubkey())
                    };
                    let valor = rng.entre(10 * UNIT, 400 * UNIT);
                    let authority = env.authority.insecure_clone();
                    if env.deploy_capital_raw(&authority, conta, valor).is_ok() {
                        destino_do_deploy = Some(dono);
                        Passo::Deploy
                    } else {
                        Passo::Nada
                    }
                }
                _ => {
                    // Sócio saca taxa, se tiver livro e caixa livre.
                    let s = rng.entre(0, 2) as usize;
                    // O livro só existe depois da primeira apuração — antes
                    // dela a conta não foi criada, e `ledger()` desembrulha um
                    // `None`.
                    let socio_key = env.socios[s].wallet.pubkey();
                    let livro = if env.svm.get_account(&fee_share_pda(&socio_key)).is_some() {
                        env.ledger(&socio_key).shares
                    } else {
                        0
                    };
                    if livro > 0 {
                        let cotas = rng.entre(1, livro);
                        // `Holder` não é `Clone`: a chave sai por
                        // `insecure_clone`, como nos testes de taxa.
                        let socio = Holder {
                            wallet: env.socios[s].wallet.insecure_clone(),
                            dom: env.socios[s].dom,
                            usdc: env.socios[s].usdc,
                        };
                        if env.redeem_fee_share_raw(&socio, cotas).is_ok() {
                            Passo::RedeemFeeShare
                        } else {
                            Passo::Nada
                        }
                    } else {
                        Passo::Nada
                    }
                }
            };

            if passo != Passo::Nada {
                passos_efetivos += 1;
            }
            *por_passo.entry(format!("{passo:?}")).or_insert(0) += 1;
            conferir_invariantes_negativas(
                &env,
                &antes,
                passo,
                &comuns,
                admitido,
                destino_do_deploy,
                rodada,
            );
        }
        sequencias += 1;
    }

    println!(
        "T66: {sequencias} sequencias x 60 rodadas, {passos_efetivos} passos \
         efetivos — 6 invariantes negativas conferidas apos cada passo"
    );
    println!("T66 cobertura por passo: {por_passo:?}");

    // -----------------------------------------------------------------------
    // O laco tem que ter EXERCIDO o codigo novo, nao so' rodado sobre ele.
    // -----------------------------------------------------------------------
    // Sem estes pisos, uma mudanca futura que fizesse `deploy_capital` sempre
    // falhar (allowlist vazia, reserva mal calculada, conta errada no harness)
    // deixaria o T66 verde e mudo — o sorteio cairia em `Nada` e ninguem
    // perceberia que a invariante 2 parou de ter o que conferir.
    for (passo, minimo) in [
        ("Deposit", 10usize),
        ("PublishNav", 10),
        ("SolicitarResgate", 5),
        ("EfetivarResgate", 5),
        ("Transfer", 5),
        ("Deploy", 5),
    ] {
        let vezes = por_passo.get(passo).copied().unwrap_or(0);
        assert!(
            vezes >= minimo,
            "T66: o passo {passo} rodou {vezes} vezes, minimo {minimo} — o fuzz \
             parou de exercer esse caminho e as invariantes ficaram sem o que \
             conferir. Cobertura: {por_passo:?}"
        );
    }
}
