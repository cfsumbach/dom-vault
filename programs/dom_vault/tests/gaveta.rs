//! Upgrade J — a gaveta com chave (D-F2-43 §6b.2, item 4 revertido em 21/09).
//!
//! A gaveta é uma conta de USDC **apontada pela mesa** (`set_gaveta_usdc`,
//! autoridade, 2/3) cuja dona é o vault 1 do Squads. O programa só lê o saldo;
//! quem move é a dona, assinando o `deposit_especial` — o P vai da gaveta ao
//! caixa por CPI, sem conta intermediária. Aqui a dona é um keypair "vault 1",
//! distinto da autoridade, como em mainnet.
#![allow(clippy::result_large_err)]

mod common;

use {
    anchor_lang::{solana_program::instruction::Instruction, InstructionData, ToAccountMetas},
    common::*,
    dom_vault::error::DomError,
    solana_keypair::Keypair,
    solana_signer::Signer,
    spl_token_2022_interface::instruction::transfer_checked,
};

/// Cofre cuja gaveta pertence a um "vault 1" que não é a autoridade.
fn cofre_com_vault_1() -> (Env, Keypair, Holder) {
    let mut env = Env::new();
    let vault1 = Keypair::new();
    env.svm.airdrop(&vault1.pubkey(), SOL).unwrap();
    let gaveta = env.criar_gaveta_de(&vault1.pubkey());
    env.gaveta = gaveta;
    let cotista = env.cotista(5_000 * UNIT);
    (env, vault1, cotista)
}

/// A dona da gaveta fecha o ciclo; a autoridade, que não é dona, não fecha.
#[test]
fn quem_fecha_e_a_dona_da_gaveta_nao_a_autoridade() {
    let (mut env, vault1, _cotista) = cofre_com_vault_1();
    env.p_na_gaveta(1_000 * UNIT);
    env.publish_nav_pela_mesa(1_200_000);

    let authority = env.authority.insecure_clone();
    let gaveta = env.gaveta;
    assert!(
        env.deposit_especial_raw(&authority, gaveta, 1_000 * UNIT)
            .is_err(),
        "a autoridade nao e' dona da gaveta: nao move o P"
    );
    let caixa_antes = env.caixa();
    assert_ok(
        env.deposit_especial_raw(&vault1, gaveta, 1_000 * UNIT),
        "fechamento pelo vault 1",
    );
    assert_eq!(
        env.caixa(),
        caixa_antes + 1_000 * UNIT,
        "o P saiu da gaveta para o caixa por CPI"
    );
    assert_eq!(
        saldo(&env.svm, &gaveta),
        0,
        "gaveta vazia depois do fechamento"
    );
    assert_eq!(env.vault().nav_fechamento, 1_100_000);
}

/// **Tudo que entra na gaveta vira P e não sai mais.** Se a dona tirar depois
/// de o P ter sido absorvido, o cofre trava (`GavetaDiminuiu`) — publish_nav,
/// aporte e fechamento recusam — até o saldo voltar. É o custo da gaveta com
/// chave, aceito pela mesa em 21/09.
#[test]
fn a_dona_tirar_da_gaveta_depois_da_absorcao_trava_o_cofre_ate_devolver() {
    let (mut env, vault1, _cotista) = cofre_com_vault_1();
    env.p_na_gaveta(1_000 * UNIT);
    env.publish_nav_pela_mesa(1_200_000); // absorveu: p_ciclo = 1.000
    assert_eq!(env.vault().p_ciclo, 1_000 * UNIT);

    // a dona manda 100 para fora (uma proposta errada do vault 1)
    let fora = env.carteira(0);
    let usdc_mint = env.usdc_mint;
    let saida = transfer_checked(
        &token_classic(),
        &env.gaveta,
        &usdc_mint,
        &fora.usdc,
        &vault1.pubkey(),
        &[],
        100 * UNIT,
        6,
    )
    .unwrap();
    let payer = env.payer.insecure_clone();
    assert_ok(
        send(&mut env.svm, &payer, &[saida], &[&vault1]),
        "a dona pode tirar — o programa nao e' autoridade",
    );

    // e o cofre trava em tudo que le a gaveta
    env.avancar_sem_oraculo(dom_vault::constants::MIN_NAV_PUBLISH_INTERVAL);
    let mesa = env.authority.insecure_clone();
    let agora = env.agora();
    assert_dom_error(
        env.publish_nav_raw(&mesa, 1_200_000, agora),
        DomError::GavetaDiminuiu,
        "publish_nav com a gaveta menor",
    );
    let novato = env.carteira(500 * UNIT);
    assert_dom_error(
        env.deposit_raw(&novato, 500 * UNIT),
        DomError::GavetaDiminuiu,
        "aporte com a gaveta menor",
    );
    let gaveta = env.gaveta;
    assert_dom_error(
        env.deposit_especial_raw(&vault1, gaveta, 900 * UNIT),
        DomError::GavetaDiminuiu,
        "fechamento com a gaveta menor",
    );
    assert!(
        env.set_gaveta_usdc_raw(&mesa, fora.usdc).is_err(),
        "trocar a gaveta com P absorvido tambem nao passa"
    );

    // devolvido, destrava
    env.p_na_gaveta(100 * UNIT);
    assert_ok(
        env.publish_nav_raw(&mesa, 1_200_000, agora),
        "publish_nav com o saldo de volta",
    );
    assert_ok(
        env.deposit_especial_raw(&vault1, gaveta, 1_000 * UNIT),
        "fechamento com o P inteiro",
    );
}

/// `set_gaveta_usdc`: só a autoridade, nunca a treasury, e só entre ciclos.
#[test]
fn set_gaveta_so_a_autoridade_nunca_a_treasury_e_so_entre_ciclos() {
    let (mut env, _vault1, _cotista) = cofre_com_vault_1();
    let outra_dona = Keypair::new();
    let outra = create_conta_usdc(
        &mut env.svm,
        &env.payer,
        &env.usdc_mint,
        &outra_dona.pubkey(),
    );

    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), SOL).unwrap();
    assert_dom_error(
        env.set_gaveta_usdc_raw(&intruso, outra),
        DomError::Unauthorized,
        "intruso apontando gaveta",
    );

    let mesa = env.authority.insecure_clone();
    assert_dom_error(
        env.set_gaveta_usdc_raw(&mesa, treasury_pda()),
        DomError::GavetaErrada,
        "a treasury como gaveta",
    );

    // entre ciclos (nada absorvido): passa, e o saldo visto zera
    assert_ok(
        env.set_gaveta_usdc_raw(&mesa, outra),
        "apontar gaveta nova entre ciclos",
    );
    assert_eq!(env.vault().gaveta_usdc, outra);
    assert_eq!(env.vault().gaveta_saldo_visto, 0);

    // com P absorvido: recusa
    env.gaveta = outra;
    env.p_na_gaveta(300 * UNIT);
    env.publish_nav_pela_mesa(1_060_000);
    assert_eq!(env.vault().p_ciclo, 300 * UNIT);
    let gaveta_velha = create_conta_usdc(
        &mut env.svm,
        &env.payer,
        &env.usdc_mint,
        &outra_dona.pubkey(),
    );
    assert_dom_error(
        env.set_gaveta_usdc_raw(&mesa, gaveta_velha),
        DomError::PConservacaoFalhou,
        "trocar a gaveta com P no indice",
    );
}

/// A instrução temporária da cerimônia regrava a lista de contas extras do
/// hook sobre a lista VIVA (a entrada TLV já existe): `update`, não `init`.
/// Em devnet (21/09) o `init` derrubou a proposta de migração inteira com
/// `TypeAlreadyExists`. Depois de regravar, a transferência com hook passa.
#[test]
fn atualizar_extra_account_meta_list_regrava_a_lista_viva() {
    let mut env = Env::new();
    let a = env.cotista(5_000 * UNIT);
    let b = env.carteira(0);
    assert_ok(
        env.atualizar_extra_account_meta_list(),
        "regravar a lista viva",
    );
    assert_ok(env.atualizar_extra_account_meta_list(), "idempotente");
    env.transfer(&a, &b.dom, &b.wallet.pubkey(), 100 * UNIT)
        .expect("hook resolve as 10 metas depois da regravacao");
    assert_eq!(saldo(&env.svm, &b.dom), 100 * UNIT);
}

/// **No J toda cota vive na ATA do dono.** O acerto é por carteira e lê UMA
/// conta: com uma segunda conta, o dono acertaria pela menor, marcaria a
/// posição em dia e a maior escaparia da taxa. O hook recusa cota entrando
/// numa conta que não é a ATA, e `acertar` recusa conta que não é a ATA.
#[test]
fn cota_so_vive_na_ata_do_dono() {
    let mut env = Env::new();
    let a = env.cotista(5_000 * UNIT);
    let b = env.carteira(0);
    // uma segunda conta de cota de B, que nao e' a ATA
    let auxiliar =
        create_conta_dom_auxiliar(&mut env.svm, &env.payer, &env.dom_mint, &b.wallet.pubkey());
    env.acertar(&a);
    assert!(
        env.transfer(&a, &auxiliar, &b.wallet.pubkey(), 100 * UNIT)
            .is_err(),
        "cota entrando em conta que nao e' a ATA: o hook recusa"
    );
    env.transfer(&a, &b.dom, &b.wallet.pubkey(), 100 * UNIT)
        .expect("na ATA passa");
    // acertar pela conta auxiliar: recusa
    let mut metas = dom_vault::accounts::Acertar {
        owner: b.wallet.pubkey(),
        vault: vault_pda(),
        posicao: posicao_pda(&b.wallet.pubkey()),
        dom_mint: env.dom_mint,
        owner_dom: auxiliar,
        dom_token_program: token_2022(),
    }
    .to_account_metas(None);
    metas[0].is_signer = true;
    let ix = Instruction::new_with_bytes(
        dom_vault::ID,
        &dom_vault::instruction::Acertar {}.data(),
        metas,
    );
    let w = b.wallet.insecure_clone();
    let payer = env.payer.insecure_clone();
    assert_dom_error(
        send(&mut env.svm, &payer, &[ix], &[&w]),
        DomError::ContaDeCotaNaoEAta,
        "acertar por conta que nao e' a ATA",
    );
}

/// **Abrir posição alheia depois de P** (blindagem da mesa, 21/09). Um holder
/// da era I tem cota mas ainda não tem posição; alguém absorve um P; um
/// terceiro abre a posição dele. A entrada nasce em `indice_ciclo` (o piso do
/// ciclo), não em `indice_p`: o ganho dele no fechamento é
/// `cotas × (indice_p − indice_ciclo)` e a conservação continua fechando.
#[test]
fn abrir_posicao_de_holder_com_cota_depois_de_p_entra_no_piso_do_ciclo() {
    let mut env = Env::new();
    // o "holder da era I": cota sem posicao (aporte + posicao apagada da VM)
    let holder = env.holder_da_era_i(10_000 * UNIT);
    let outro = env.cotista(10_000 * UNIT);
    assert!(
        env.posicao_de(&holder.wallet.pubkey()).is_none(),
        "sem posicao ainda"
    );
    // um P e' absorvido ANTES de a posicao existir
    env.p_na_gaveta(1_000 * UNIT);
    env.publish_nav_pela_mesa(1_050_000);
    let v = env.vault();
    assert!(v.indice_p > 0 && v.indice_ciclo == 0);
    // um terceiro (o pagador do harness) abre a posicao do holder
    env.abrir_posicao(&holder.wallet.pubkey());
    let pos = env.posicao_de(&holder.wallet.pubkey()).unwrap();
    assert_eq!(
        pos.indice_entrada, v.indice_ciclo,
        "com cota, a entrada e' o piso do ciclo — nao o indice de agora"
    );
    let ganho_holder = dom_vault::indice::ganho(
        env.saldo_dom(&holder),
        pos.indice_entrada,
        v.indice_ciclo,
        v.indice_p,
    )
    .unwrap();
    let ganho_outro = dom_vault::indice::ganho(
        env.saldo_dom(&outro),
        env.posicao_de(&outro.wallet.pubkey())
            .unwrap()
            .indice_entrada,
        v.indice_ciclo,
        v.indice_p,
    )
    .unwrap();
    assert_eq!(
        ganho_holder,
        500 * UNIT,
        "metade do P e' dele: 10.000 × 1.000/20.000"
    );
    assert!(
        (ganho_holder + ganho_outro).abs_diff(1_000 * UNIT) < 4,
        "conservacao"
    );
    // e sem cota, a entrada continua sendo o indice de agora
    let novo = env.carteira(0);
    assert_eq!(
        env.posicao_de(&novo.wallet.pubkey())
            .unwrap()
            .indice_entrada,
        v.indice_p
    );
}

/// **`set_gaveta_usdc` grava o saldo visto = saldo atual** (mesa, 21/09): o que a
/// conta já tem ao ser apontada não é P; só o delta posterior vira P.
#[test]
fn apontar_gaveta_com_saldo_nao_cria_p() {
    let mut env = Env::new();
    let _c = env.cotista(5_000 * UNIT);
    let dona = Keypair::new();
    let conta = create_conta_usdc(&mut env.svm, &env.payer, &env.usdc_mint, &dona.pubkey());
    let (mint, auth) = (env.usdc_mint, env.usdc_authority.insecure_clone());
    mint_tokens(
        &mut env.svm,
        &env.payer,
        &token_classic(),
        &mint,
        &auth,
        &conta,
        700 * UNIT,
    );
    env.set_gaveta_usdc(conta);
    env.gaveta = conta;
    let v = env.vault();
    assert_eq!(
        v.gaveta_saldo_visto,
        700 * UNIT,
        "o saldo existente e' o ponto zero"
    );
    assert_eq!(v.p_ciclo, 0);
    env.sincronizar_gaveta();
    assert_eq!(env.vault().p_ciclo, 0, "ler de novo nao cria P");
    assert_eq!(env.vault().indice_p, 0);
    env.p_na_gaveta(300 * UNIT);
    env.sincronizar_gaveta();
    assert_eq!(
        env.vault().p_ciclo,
        300 * UNIT,
        "so' o delta posterior e' P"
    );
}
