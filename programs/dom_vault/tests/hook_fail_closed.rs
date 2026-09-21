//! Grupo C da matriz — **hook fail-closed** (T19–T24).
//!
//! Cada teste ataca uma frente do D5. Todos rodam em LiteSVM com o Token-2022
//! real (o `.so` que a litesvm embarca), não com um mock do hook: o que valida
//! é o mesmo programa que roda em devnet.
//!
//! Ordem das checagens do hook é deliberada e os testes dependem dela — mint,
//! depois contas do mint, depois `transferring`, depois whitelist, depois cap.
//! É o que torna cada rejeição atribuível a uma causa só (D13).

#![allow(clippy::result_large_err)]

mod common;

use {
    anchor_lang::{prelude::Pubkey, solana_program::instruction::AccountMeta},
    common::*,
    dom_vault::error::DomError,
    solana_keypair::Keypair,
    solana_signer::Signer,
    spl_token_2022_interface::{
        extension::{transfer_hook, BaseStateWithExtensions, StateWithExtensions},
        instruction::{set_authority, AuthorityType},
        state::Mint as MintState,
    },
    spl_transfer_hook_interface::instruction::execute_with_extra_account_metas,
};

/// Aporte padrão dos testes: 1.000 USDC. Ao NAV da gênese vira 1.000 cotas, o
/// que deixa as contas de cap em números redondos.
const APORTE: u64 = 1_000 * UNIT;

// ---------------------------------------------------------------------------
// T19 — mint estranho
// ---------------------------------------------------------------------------

/// O atacante cria o próprio mint apontando o hook para o `dom_vault` e invoca
/// `Execute` direto, para envenenar estado ou descobrir comportamento. O mint
/// não é o do DOM — rejeita antes de qualquer outra leitura (E00 / D5.1).
#[test]
fn t19_hook_rejeita_mint_estranho() {
    let mut env = Env::new();

    let atacante = Keypair::new();
    env.svm.airdrop(&atacante.pubkey(), 10 * SOL).unwrap();

    let mint_falso = Keypair::new();
    create_mint_com_hook(
        &mut env.svm,
        &env.payer,
        &mint_falso,
        &atacante.pubkey(),
        Some(atacante.pubkey()),
        // aponta para o NOSSO programa: é esse o ataque
        Some(dom_vault::ID),
    );
    let origem = create_conta_dom(
        &mut env.svm,
        &env.payer,
        &mint_falso.pubkey(),
        &atacante.pubkey(),
    );
    let destino = create_conta_dom(
        &mut env.svm,
        &env.payer,
        &mint_falso.pubkey(),
        &atacante.pubkey(),
    );
    mint_tokens(
        &mut env.svm,
        &env.payer,
        &token_2022(),
        &mint_falso.pubkey(),
        &atacante,
        &origem,
        1_000,
    );

    let ix = execute_with_extra_account_metas(
        &dom_vault::ID,
        &origem,
        &mint_falso.pubkey(),
        &destino,
        &atacante.pubkey(),
        &validation_pda(&mint_falso.pubkey()),
        &[
            AccountMeta::new_readonly(vault_pda(), false),
            AccountMeta::new_readonly(whitelist_pda(&atacante.pubkey()), false),
            AccountMeta::new_readonly(whitelist_pda(&atacante.pubkey()), false),
            AccountMeta::new(posicao_pda(&atacante.pubkey()), false),
            AccountMeta::new(posicao_pda(&atacante.pubkey()), false),
        ],
        1_000,
    );

    assert_dom_error(
        send(&mut env.svm, &env.payer, &[ix], &[]),
        DomError::UnknownMint,
        "T19 mint estranho",
    );
}

// ---------------------------------------------------------------------------
// T20 — contas de outro mint
// ---------------------------------------------------------------------------

/// Mint certo no lugar do mint, contas de token de outro mint nas pontas. Sem a
/// checagem, o hook leria saldo e supply de universos diferentes (D5.2).
#[test]
fn t20_hook_rejeita_contas_de_outro_mint() {
    let mut env = Env::new();

    let atacante = Keypair::new();
    env.svm.airdrop(&atacante.pubkey(), 10 * SOL).unwrap();

    let outro_mint = Keypair::new();
    create_mint_com_hook(
        &mut env.svm,
        &env.payer,
        &outro_mint,
        &atacante.pubkey(),
        Some(atacante.pubkey()),
        Some(dom_vault::ID),
    );
    let origem = create_conta_dom(
        &mut env.svm,
        &env.payer,
        &outro_mint.pubkey(),
        &atacante.pubkey(),
    );
    let destino = create_conta_dom(
        &mut env.svm,
        &env.payer,
        &outro_mint.pubkey(),
        &atacante.pubkey(),
    );

    let ix = execute_with_extra_account_metas(
        &dom_vault::ID,
        &origem,
        &env.dom_mint, // o mint do DOM, para passar da D5.1
        &destino,
        &atacante.pubkey(),
        &validation_pda(&env.dom_mint),
        &[
            AccountMeta::new_readonly(vault_pda(), false),
            AccountMeta::new_readonly(whitelist_pda(&atacante.pubkey()), false),
            AccountMeta::new_readonly(whitelist_pda(&atacante.pubkey()), false),
            AccountMeta::new(posicao_pda(&atacante.pubkey()), false),
            AccountMeta::new(posicao_pda(&atacante.pubkey()), false),
        ],
        1_000,
    );

    assert_dom_error(
        send(&mut env.svm, &env.payer, &[ix], &[]),
        DomError::TokenAccountMintMismatch,
        "T20 contas de outro mint",
    );
}

// ---------------------------------------------------------------------------
// T21 — invocação direta, sem transferência
// ---------------------------------------------------------------------------

/// Tudo legítimo — mint do DOM, contas do mint, as duas carteiras na whitelist
/// — mas ninguém está transferindo. `transferring` só é `true` durante o CPI do
/// Token-2022. Invocação direta rejeita (D5.3).
#[test]
fn t21_hook_rejeita_invocacao_direta() {
    let mut env = Env::new();
    let origem = env.cotista(APORTE);
    let destino = env.carteira(0);

    let ix = execute_with_extra_account_metas(
        &dom_vault::ID,
        &origem.dom,
        &env.dom_mint,
        &destino.dom,
        &origem.wallet.pubkey(),
        &validation_pda(&env.dom_mint),
        &[
            AccountMeta::new_readonly(vault_pda(), false),
            AccountMeta::new_readonly(whitelist_pda(&origem.wallet.pubkey()), false),
            AccountMeta::new_readonly(whitelist_pda(&destino.wallet.pubkey()), false),
            AccountMeta::new(posicao_pda(&destino.wallet.pubkey()), false),
            AccountMeta::new(posicao_pda(&origem.wallet.pubkey()), false),
        ],
        1_000,
    );

    assert_dom_error(
        send(&mut env.svm, &env.payer, &[ix], &[]),
        DomError::NotTransferring,
        "T21 invocacao direta",
    );
}

// ---------------------------------------------------------------------------
// T22 — whitelist do destino não inicializada
// ---------------------------------------------------------------------------

/// Transferência real para uma carteira que nunca foi habilitada: a conta de
/// whitelist do destino **não existe**. Ausência é rejeição, não omissão (D5).
///
/// O mesmo teste habilita o destino no fim e repete a transferência. Sem esse
/// contra-teste, um erro de encanamento (conta extra faltando, resolução
/// errada) passaria por "fail-closed funcionando".
#[test]
fn t22_hook_rejeita_whitelist_ausente_no_destino() {
    let mut env = Env::new();
    let origem = env.cotista(APORTE);

    // Carteira sem `update_whitelist`: conta de DOM existe, whitelist não.
    let destino_wallet = Keypair::new();
    let destino = create_conta_dom(
        &mut env.svm,
        &env.payer,
        &env.dom_mint,
        &destino_wallet.pubkey(),
    );

    assert!(
        env.svm
            .get_account(&whitelist_pda(&destino_wallet.pubkey()))
            .is_none_or(|a| a.data.is_empty()),
        "T22: a whitelist do destino nao deveria existir ainda"
    );

    assert_dom_error(
        env.transfer(&origem, &destino, &destino_wallet.pubkey(), 1_000),
        DomError::NotWhitelisted,
        "T22 whitelist ausente",
    );

    // Contra-teste: com a whitelist criada, a mesma transferência passa.
    env.whitelist(&destino_wallet.pubkey(), true);
    // Upgrade J: whitelist ativa mas SEM posicao no indice — o hook recusa com
    // erro proprio (a carteira nao pode entrar num lote sem registrar o indice).
    assert_dom_error(
        env.transfer(&origem, &destino, &destino_wallet.pubkey(), 1_000),
        DomError::PosicaoDoCotistaAusente,
        "T22 whitelist ativa sem posicao (Upgrade J)",
    );
    env.abrir_posicao(&destino_wallet.pubkey());
    assert_ok(
        env.transfer(&origem, &destino, &destino_wallet.pubkey(), 1_000),
        "T22 contra-teste com whitelist ativa e posicao aberta",
    );

    // E desabilitar volta a bloquear — `active = false` não é "sem opinião".
    env.whitelist(&destino_wallet.pubkey(), false);
    assert_dom_error(
        env.transfer(&origem, &destino, &destino_wallet.pubkey(), 1_000),
        DomError::NotWhitelisted,
        "T22 whitelist desativada",
    );
}

// ---------------------------------------------------------------------------
// T23 — troca do transfer_hook authority por chave não autorizada
// ---------------------------------------------------------------------------

/// Se um terceiro puder trocar a autoridade do hook — ou o próprio programa do
/// hook — a whitelist vira decoração (D2). Quem barra aqui é o Token-2022, e é
/// exatamente isso que o teste prova: as duas tentativas falham e a extensão do
/// mint continua apontando para o `dom_vault`.
#[test]
fn t23_troca_de_hook_por_chave_nao_autorizada_falha() {
    let mut env = Env::new();

    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), 10 * SOL).unwrap();

    // 1. trocar a autoridade do hook
    let ix = set_authority(
        &token_2022(),
        &env.dom_mint,
        Some(&intruso.pubkey()),
        AuthorityType::TransferHookProgramId,
        &intruso.pubkey(),
        &[],
    )
    .unwrap();
    let intruso_ref = intruso.insecure_clone();
    assert!(
        send(&mut env.svm, &env.payer, &[ix], &[&intruso_ref]).is_err(),
        "T23: intruso conseguiu trocar o transfer_hook authority"
    );

    // 2. apontar o hook para outro programa
    let ix = transfer_hook::instruction::update(
        &token_2022(),
        &env.dom_mint,
        &intruso.pubkey(),
        &[],
        Some(Pubkey::new_unique()),
    )
    .unwrap();
    assert!(
        send(&mut env.svm, &env.payer, &[ix], &[&intruso_ref]).is_err(),
        "T23: intruso conseguiu trocar o programa do hook"
    );

    // O mint continua apontando para o dom_vault.
    let conta = env.svm.get_account(&env.dom_mint).unwrap();
    let estado = StateWithExtensions::<MintState>::unpack(&conta.data).unwrap();
    let hook = estado
        .get_extension::<transfer_hook::TransferHook>()
        .unwrap();
    let program_id: Option<Pubkey> = hook.program_id.into();
    assert_eq!(
        program_id,
        Some(dom_vault::ID),
        "T23: o hook do mint mudou depois das tentativas"
    );
}

// ---------------------------------------------------------------------------
// T24 — contas do programa isentas do cap
// ---------------------------------------------------------------------------

/// Com `cap_enforced` ativo, o escrow do programa recebe 30% do supply e
/// **passa**: não é carteira de cotista, é o cofre (D2/D12).
///
/// O contra-teste na mesma execução manda os mesmos 30% para uma carteira comum
/// e exige rejeição — senão "passou" só provaria que o cap está morto.
#[test]
fn t24_contas_do_programa_sao_isentas_do_cap() {
    let mut env = Env::new();
    let origem = env.cotista(APORTE);
    let comum = env.carteira(0);
    let escrow = create_conta_dom(&mut env.svm, &env.payer, &env.dom_mint, &escrow_pda());
    env.enable_cap();

    let supply = env.supply_dom();
    let acima_do_cap = supply / 10 * 3; // 30% do supply

    assert_ok(
        env.transfer(&origem, &escrow, &escrow_pda(), acima_do_cap),
        "T24 escrow do programa acima de 25%",
    );

    assert_dom_error(
        env.transfer(&origem, &comum.dom, &comum.wallet.pubkey(), acima_do_cap),
        DomError::CapExceeded,
        "T24 contra-teste: carteira comum acima de 25%",
    );
}

// ---------------------------------------------------------------------------
// A3 — auto-transferência (origem == destino)
// ---------------------------------------------------------------------------

/// Anchor 1.x falha por padrão quando a mesma conta mutável aparece duas vezes
/// numa instrução (A3). O Token-2022 permite `source == destination`, e nesse
/// caso o hook recebe a mesma conta nas duas pontas — junto com a mesma conta de
/// whitelist. O teste existe para que isso não vire uma falha descoberta em
/// produção.
///
/// Cotista abaixo do cap: o saldo não muda, então o cap não deve disparar.
#[test]
fn a3_auto_transferencia_nao_quebra_o_hook() {
    let mut env = Env::new();
    let grande = env.cotista(APORTE);
    let pequeno = env.carteira(0);

    // 20% do supply para o pequeno, com o cap ainda desligado.
    let quinto = env.supply_dom() / 5;
    assert_ok(
        env.transfer(&grande, &pequeno.dom, &pequeno.wallet.pubkey(), quinto),
        "A3 preparo",
    );
    env.enable_cap();

    assert_ok(
        env.transfer(&pequeno, &pequeno.dom, &pequeno.wallet.pubkey(), 1_000),
        "A3 auto-transferencia abaixo do cap",
    );
}

// ---------------------------------------------------------------------------
// T68 — CANÁRIO do Token-2022 (auditoria F4, rodada 4)
// ---------------------------------------------------------------------------

/// T68 — **canário na mina.** Afirma a ordem de execução do Token-2022 da qual o
/// cap deste programa depende.
///
/// A premissa (D4): quando o `Execute` do hook roda, o Token-2022 **já gravou**
/// `source.amount -= x` e `destination.amount += x`. Por isso `execute.rs` lê
/// `destination.amount` direto contra `mint.supply`, sem somar `amount` — somar
/// seria double-count.
///
/// A premissa é sobre **programa de terceiro**, não sobre invariante da runtime.
/// Se um Token-2022 futuro passar a chamar o hook **antes** de gravar, o cap
/// fica frouxo por exatamente `amount`: silenciosamente, sem erro e sem log.
///
/// Este teste discrimina as duas ordens. O cenário é escolhido para que a
/// resposta certa dependa **só** de qual saldo o hook enxerga:
///
/// | | destino | % do supply | veredito do cap |
/// |---|---|---|---|
/// | **pré-gravação** | 200 | 20,0% | passaria |
/// | **pós-gravação** | 300 | 30,0% | **rejeita** |
///
/// Se este teste falhar com "esperava CapExceeded, veio Ok", a leitura mudou de
/// lado e o cap está furado. Não é regressão nossa: é aviso de que o Token-2022
/// mudou debaixo do programa. Ver a nota do relatório F4-03.
#[test]
fn t68_canario_o_hook_le_o_saldo_apos_a_gravacao() {
    let mut env = Env::new();
    let grande = env.cotista(800 * UNIT);
    let pequeno = env.cotista(200 * UNIT);
    let supply = env.supply_dom();
    assert_eq!(supply, 1_000 * UNIT);

    env.enable_cap();

    let amount = 100 * UNIT;
    let antes = env.saldo_dom(&pequeno);
    let depois_esperado = antes + amount;

    // As duas leituras, explícitas — o teste documenta a discriminação em vez de
    // depender de quem lê o código adivinhar por que os números são estes.
    assert!(
        antes as u128 * 100 <= supply as u128 * 25,
        "pre-gravacao ({antes} de {supply}) tem que estar DENTRO do cap, \
         senao o teste nao discrimina as duas ordens"
    );
    assert!(
        depois_esperado as u128 * 100 > supply as u128 * 25,
        "pos-gravacao ({depois_esperado} de {supply}) tem que estar FORA do cap"
    );

    // O veredito observado tem que ser o da leitura PÓS-gravação.
    assert_dom_error(
        env.transfer(&grande, &pequeno.dom, &pequeno.wallet.pubkey(), amount),
        DomError::CapExceeded,
        "T68 CANARIO: o hook leu o saldo PRE-gravacao — a ordem do Token-2022 \
         mudou e o cap esta frouxo por `amount`",
    );

    // E a transferência não deixou rastro: rejeição reverte a transação inteira.
    assert_eq!(
        env.saldo_dom(&pequeno),
        antes,
        "T68 saldo do destino intacto"
    );
    assert_eq!(env.supply_dom(), supply, "T68 supply intacto");
}
