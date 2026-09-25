//! Grupos I e K da matriz — **autoridade** (T54, T55) e **configuração de
//! sócios** (T57–T60).
//!
//! A invariante da D10 é testada nos **dois** pontos de escrita. Testar só o
//! `initialize` provaria pouco: a trava seria contornável em dois passos —
//! configura distinto, atualiza para duplicado.

#![allow(clippy::result_large_err)]

mod common;

use {
    common::*,
    dom_vault::{
        constants::{NAV_GENESIS, RESERVE_BPS},
        error::DomError,
        events::{SociosUpdated, WhitelistUpdated},
    },
    solana_keypair::Keypair,
    solana_signer::Signer,
};

// ---------------------------------------------------------------------------
// T58 — o que o initialize grava
// ---------------------------------------------------------------------------

/// Gênese: NAV e HWM em 1,000000, cap desligado, reserva em 10%, âncora de ciclo
/// carimbada, fila vazia e três sócios distintos.
#[test]
fn t58_initialize_grava_a_genese() {
    // Sem a publicação inicial: este teste afirma o que o `initialize` **grava**,
    // e o `Env::new` normal já publica NAV logo depois (o `deposit` passou a
    // exigir isso — decisão da mesa de 2026-08-20, T80).
    let env = Env::sem_nav_publicado();
    let vault = env.vault();

    assert_eq!(vault.nav, NAV_GENESIS, "NAV de genese");
    assert_eq!(
        vault.delta_lucro_por_cota, 0,
        "genese nao tem distribuicao atras dela"
    );
    assert_eq!(vault.lucro_sacavel_restante, 0, "janela fechada na genese");
    assert_eq!(vault.ultima_distribuicao_ts, 0, "nunca distribuiu");
    assert_eq!(vault.nav_ts, 0, "nenhum NAV publicado ainda");
    assert!(!vault.cap_enforced, "cap desligado na genese (D3)");
    assert!(!vault.paused);
    assert_eq!(vault.reserve_bps, RESERVE_BPS, "reserva de 10% (D7)");
    assert_eq!(vault.cotas_travadas_resgate, 0);
    assert_eq!(vault.proximo_pedido_id, 0, "o primeiro pedido leva o id 0");
    assert_eq!(vault.resgates_em_atraso, 0);
    assert_eq!(
        vault.endereco_resgate,
        anchor_lang::prelude::Pubkey::default(),
        "nasce vazio, como a allowlist — capital novo nasce preso dos dois lados"
    );
    assert_eq!(
        vault.layout_version,
        dom_vault::constants::LAYOUT_VERSION,
        "a versao do layout e' carimbada na genese"
    );

    assert_eq!(vault.authority, env.authority.pubkey());
    assert_eq!(vault.nav_oracle, env.oracle.pubkey(), "oraculo separado");
    for (i, socio) in env.socios.iter().enumerate() {
        assert_eq!(vault.socios[i], socio.wallet.pubkey());
    }
    assert_ne!(vault.socios[0], vault.socios[1]);
    assert_ne!(vault.socios[0], vault.socios[2]);
    assert_ne!(vault.socios[1], vault.socios[2]);
}

// ---------------------------------------------------------------------------
// T57 / T59 / T60 — sócios distintos, nos dois pontos de escrita
// ---------------------------------------------------------------------------

/// T59 — atualizar para carteiras duplicadas rejeita, nas três combinações
/// possíveis de par. Uma checagem que só olhasse o primeiro par deixaria as
/// outras duas passarem.
#[test]
fn t59_update_socios_para_duplicados_rejeita() {
    let mut env = Env::new();
    let originais = env.vault().socios;

    let a = Keypair::new().pubkey();
    let b = Keypair::new().pubkey();
    let authority = env.authority.insecure_clone();

    for (i, duplicado) in [[a, a, b], [a, b, a], [b, a, a]].into_iter().enumerate() {
        assert_dom_error(
            env.update_socios_raw(&authority, duplicado),
            DomError::DuplicateSocio,
            &format!("T59 combinacao {i}"),
        );
        assert_eq!(env.vault().socios, originais, "socios nao mudaram");
    }

    // Contra-teste: três distintos passam e o evento sai.
    let c = Keypair::new().pubkey();
    let meta = assert_ok(
        env.update_socios_raw(&authority, [a, b, c]),
        "T59 contra-teste com tres distintos",
    );
    assert!(tem_evento::<SociosUpdated>(&meta));
    assert_eq!(env.vault().socios, [a, b, c]);
}

/// T60 — atualizar sócios por não-autoridade rejeita.
#[test]
fn t60_update_socios_por_nao_autoridade_rejeita() {
    let mut env = Env::new();
    let originais = env.vault().socios;

    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), SOL).unwrap();

    let novos = [
        Keypair::new().pubkey(),
        Keypair::new().pubkey(),
        Keypair::new().pubkey(),
    ];
    assert_dom_error(
        env.update_socios_raw(&intruso, novos),
        DomError::Unauthorized,
        "T60 update por intruso",
    );
    assert_eq!(env.vault().socios, originais);
}

/// T57 — a mesma trava no `initialize`. Como o `Env` já inicializa o cofre, o
/// teste sobe um ambiente cru e chama a instrução direto.
#[test]
fn t57_initialize_com_socios_duplicados_rejeita() {
    let socio = Keypair::new().pubkey();
    let outro = Keypair::new().pubkey();

    let resultado = Env::tentar_initialize([socio, socio, outro]);
    assert!(
        resultado.is_err(),
        "T57: initialize com socios duplicados deveria falhar"
    );

    // Contra-teste: distintos passam.
    let terceiro = Keypair::new().pubkey();
    assert!(
        Env::tentar_initialize([socio, outro, terceiro]).is_ok(),
        "T57 contra-teste: tres distintos"
    );
}

// ---------------------------------------------------------------------------
// T54 / T55 — autoridade e evento da whitelist
// ---------------------------------------------------------------------------

/// T55 — `update_whitelist` bem-sucedido emite evento, tanto ao habilitar
/// quanto ao desabilitar.
#[test]
fn t55_update_whitelist_emite_evento() {
    let mut env = Env::new();
    let carteira = Keypair::new().pubkey();

    let meta = env.whitelist(&carteira, true);
    assert!(tem_evento::<WhitelistUpdated>(&meta), "evento ao habilitar");

    let meta = env.whitelist(&carteira, false);
    assert!(
        tem_evento::<WhitelistUpdated>(&meta),
        "evento ao desabilitar"
    );
}

/// T54 — as instruções privilegiadas, todas, rejeitam quem não é a autoridade.
///
/// Este teste é a rede: cada instrução tem a sua checagem, mas é aqui que a
/// lista inteira é varrida de uma vez. Instrução privilegiada nova entra aqui.
#[test]
fn t54_instrucoes_privilegiadas_rejeitam_nao_autoridade() {
    let mut env = Env::new();
    let cotista = env.cotista(20_000 * UNIT);

    // Um pedido de resgate REAL. Sem ele, o `efetivar` seria barrado pela conta
    // ausente (`3012`) antes de chegar na checagem de autoridade — e o teste
    // passaria provando a coisa errada.
    let pagadora = env.caixa_da_autoridade(50_000 * UNIT);
    env.update_endereco_resgate(pagadora);
    let pedido = env.solicitar_resgate(&cotista, 5_000 * UNIT);

    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), 10 * SOL).unwrap();
    let carteira = Keypair::new().pubkey();

    assert_dom_error(
        env.update_whitelist_raw(&intruso, &carteira, true),
        DomError::Unauthorized,
        "T54 update_whitelist",
    );
    assert_dom_error(
        env.enable_cap_raw(&intruso),
        DomError::Unauthorized,
        "T54 enable_cap",
    );
    assert_dom_error(env.pause_raw(&intruso), DomError::Unauthorized, "T54 pause");
    assert_dom_error(
        env.efetivar_resgate_raw(&intruso, pedido, 1, cotista.usdc),
        DomError::Unauthorized,
        "T54 efetivar_resgate_capital",
    );
    let origem = env.caixa_da_autoridade(0);
    assert!(
        env.deposit_especial_raw(&intruso, origem, 1).is_err(),
        "T54 deposit_especial por intruso"
    );
    // Upgrade J: a gaveta so' a autoridade aponta; as duas temporarias idem.
    let conta_usdc_qualquer = env.socios[0].usdc;
    assert_dom_error(
        env.set_gaveta_usdc_raw(&intruso, conta_usdc_qualquer),
        DomError::Unauthorized,
        "T54 set_gaveta_usdc",
    );
    // Upgrade K: a caneta nova entra na varredura JUNTO com o teste de recusa
    // dela — nunca depois. As duas temporarias do J sairam (D-F2-45 §4).
    assert_dom_error(
        env.ajustar_deployed_usdc_raw(&intruso, 1, "teste de intruso"),
        DomError::Unauthorized,
        "T54 ajustar_deployed_usdc",
    );
    assert_dom_error(
        env.update_socios_raw(
            &intruso,
            [
                Keypair::new().pubkey(),
                Keypair::new().pubkey(),
                Keypair::new().pubkey(),
            ],
        ),
        DomError::Unauthorized,
        "T54 update_socios",
    );

    // Bloco 3 da F2 — a porta operacional.
    let ops = Keypair::new().pubkey();
    assert_dom_error(
        env.update_deploy_allowlist_raw(&intruso, 0, ops),
        DomError::Unauthorized,
        "T54 update_deploy_allowlist",
    );
    let conta_qualquer = env.socios[0].usdc;
    assert_dom_error(
        env.deploy_capital_raw(&intruso, conta_qualquer, 1),
        DomError::Unauthorized,
        "T54 deploy_capital",
    );

    // Minimos ajustaveis — a 12a caneta, mais a migracao temporaria.
    assert_dom_error(
        env.update_min_deposit_raw(&intruso, 1_000_000),
        DomError::Unauthorized,
        "T54 update_min_deposit",
    );

    // Upgrade E — a mesa passou a votar VALOR, e por isso a porta que grava
    // valor e' agora uma caneta como as outras. O intruso e' barrado no
    // `has_one`, antes de qualquer limite: o teto do parametro protege a mesa
    // de si mesma, nao protege o cofre de estranho — quem faz isso e' isto aqui.
    assert_dom_error(
        env.ajustar_parametro_raw(
            &intruso,
            dom_vault::instructions::parametros::Parametro::MinSaqueLucro,
            1,
        ),
        DomError::Unauthorized,
        "T54 ajustar_parametro",
    );

    // `unpause` tem o mesmo `has_one` do `pause`, e é instrução separada no IDL
    // — logo, item separado da varredura.
    assert_dom_error(
        env.unpause_raw(&intruso),
        DomError::Unauthorized,
        "T54 unpause",
    );

    // `publish_nav` é do oráculo, não da autoridade — erro próprio.
    let agora = env.agora();
    assert_dom_error(
        env.publish_nav_raw(&intruso, 1_100_000, agora),
        DomError::NotOracle,
        "T54 publish_nav",
    );

    // Nada mudou.
    let vault = env.vault();
    assert!(!vault.cap_enforced);
    assert!(!vault.paused);
    assert_eq!(vault.nav, NAV_GENESIS);
}

// ---------------------------------------------------------------------------
// T65 — a sétima privilegiada (auditoria F4-01, pergunta B)
// ---------------------------------------------------------------------------

/// T65 — `initialize_extra_account_meta_list` também rejeita não-autoridade.
///
/// Ela ficou de fora do T54 porque é de bootstrap: roda uma vez, no `Env::new`.
/// Mas é uma das cinco privilegiadas do mandato, e "roda uma vez" não é motivo
/// para não ter teste — é motivo para o teste ser este, contra o cofre já
/// inicializado.
///
/// O `has_one = authority` do `vault` é avaliado antes do handler, então o erro
/// vem como `Unauthorized` e não como "conta já existe".
#[test]
fn t65_initialize_extra_account_meta_list_rejeita_nao_autoridade() {
    use anchor_lang::{solana_program::instruction::Instruction, InstructionData, ToAccountMetas};

    let mut env = Env::new();
    let intruso = Keypair::new();
    env.svm.airdrop(&intruso.pubkey(), 10 * SOL).unwrap();

    let ix = Instruction::new_with_bytes(
        dom_vault::ID,
        &dom_vault::instruction::InitializeExtraAccountMetaList {}.data(),
        dom_vault::accounts::InitializeExtraAccountMetaList {
            payer: intruso.pubkey(),
            authority: intruso.pubkey(),
            vault: vault_pda(),
            mint: env.dom_mint,
            extra_account_meta_list: validation_pda(&env.dom_mint),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
    );

    let payer = env.payer.insecure_clone();
    let resultado = send(&mut env.svm, &payer, &[ix], &[&intruso]);
    assert_dom_error(
        resultado,
        DomError::Unauthorized,
        "T65 initialize_extra_account_meta_list por intruso",
    );
}

// ---------------------------------------------------------------------------
// T89 — a varredura que não deixa a lista envelhecer (F2-04)
// ---------------------------------------------------------------------------

/// T89 — **a lista de privilegiadas é derivada do IDL, não escrita à mão.**
///
/// Por que este teste existe: o dossiê da F1 afirmava "as **cinco** instruções
/// privilegiadas" e nomeava `initialize`, `initialize_extra_account_meta_list`,
/// `update_whitelist`, `deposit_especial` e `efetivar_resgate_capital`. A varredura
/// do código, refeita no bloco 4 da F2, mostrou que `pause`, `unpause`,
/// `enable_cap` e `update_socios` **também** exigem a autoridade com `has_one` —
/// quatro instruções que a lista nomeada não via. O comentário do próprio T65 já
/// dizia "a sétima privilegiada", com a contagem andando sozinha.
///
/// Lista nomeada envelhece em silêncio. Este teste faz o oposto: colhe do **IDL
/// do build** toda instrução que pede a conta `authority` como signatária, e
/// compara com o conjunto que a suíte de fato exerce. **Instrução privilegiada
/// nova sem teste de recusa quebra aqui**, com o nome dela na mensagem.
#[test]
fn t89_a_lista_de_privilegiadas_sai_do_idl() {
    // ⚠️ O VERSIONADO, e nao o artefato de build — descoberto em 14/09.
    //
    // Era `CARGO_TARGET_TMPDIR/../idl/`, que resolve para `target/idl/` — a
    // pasta que o `anchor idl build` escreve. No repositorio de trabalho ela
    // existe sempre, porque o build roda o tempo todo. **Num clone limpo do
    // espelho publico, nao existe** — e a suite inteira deixa de compilar.
    //
    // O espelho versiona o IDL em `idl/dom_vault.json`, na raiz, e a CI cobra
    // que os dois sejam iguais (`snapshot_do_idl`). Ler o versionado da o mesmo
    // resultado nos dois repositorios, e faz o clone limpo funcionar — que e' o
    // caminho que a carta ao investidor manda seguir.
    const IDL: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../idl/dom_vault.json"
    ));

    let idl: serde_json::Value = serde_json::from_str(IDL).unwrap();

    // Colhido do IDL: toda instrução com uma conta `authority` signatária.
    let mut do_idl: Vec<String> = idl["instructions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|ix| {
            ix["accounts"].as_array().unwrap().iter().any(|c| {
                c["name"].as_str() == Some("authority") && c["signer"].as_bool() == Some(true)
            })
        })
        .map(|ix| ix["name"].as_str().unwrap().to_string())
        .collect();
    do_idl.sort();

    // Exercidas pela suíte, com teste de recusa nomeado. **Uma linha por
    // instrução, e cada linha tem que apontar um teste que existe.**
    //
    // `sacar_lucro` NAO entra: quem assina e' o cotista, como no
    // `redeem_fee_share`. Instrucao nova nem sempre e' caneta nova (D-F2-09).
    let exercidas = vec![
        // `deposit_especial` SAIU daqui no J: quem assina e' a DONA DA GAVETA
        // (vault 1), nao a `authority` — o privilegio vem do `set_gaveta_usdc`
        // (D-F2-43 §6b.2 item 4). A recusa dela esta em gaveta.rs.
        ("deploy_capital", "T54 / T81"),
        ("enable_cap", "T54 / T11b"),
        // `initialize` é o único que assina como autoridade SEM `has_one`: ele
        // **grava** `vault.authority`, então não há contra o que comparar. É
        // gênese, roda uma vez, e quem a protege é a cerimônia — não o programa.
        ("initialize", "genese — sem has_one, por desenho"),
        ("initialize_extra_account_meta_list", "T65"),
        // **TEMPORARIA** — sai antes do snapshot da firma (D-F2-08). Enquanto
        // existir, a contagem e' 13; quando sair, volta a 12. Este teste e' o
        // lembrete: a linha abaixo sai junto com a instrucao, e o total quebra
        // se alguem esquecer de um dos dois.
        ("efetivar_resgate_capital", "T54 / resgate_capital.rs"),
        ("pause", "T54 / T49"),
        ("set_nav_oracle", "T54"),
        // `D-F2-34` — a caneta que NOMEIA o porteiro da whitelist. So' a mesa,
        // 2/3. `Pubkey::default()` destitui, e por isso nao ha' instrucao de
        // revogacao separada — que seria a que ninguem testa.
        ("set_whitelist_operator", "T54 / porteiro.rs"),
        ("unpause", "T54 / T49"),
        ("update_endereco_resgate", "T54 / resgate_capital.rs"),
        ("update_deploy_allowlist", "T54 / T81b"),
        ("update_min_deposit", "T54 / T91"),
        ("ajustar_parametro", "T54 / parametros.rs"),
        // **TEMPORARIA** — Upgrade F. Sai no upgrade seguinte, e esta linha sai
        // junto: a contagem quebra se so' uma das duas for embora.
        ("update_socios", "T54"),
        ("update_whitelist", "T54"),
        // Upgrade J (D-F2-43): a caneta que aponta a gaveta, e as duas
        // TEMPORARIAS da cerimonia — saem no upgrade seguinte, com as linhas.
        ("set_gaveta_usdc", "T54 / gaveta.rs"),
        // Upgrade K (D-F2-45): a ferramenta que corrige o custo de capital.
        ("ajustar_deployed_usdc", "T54 / ajustar_deployed.rs"),
    ];
    let mut nomes_exercidos: Vec<String> = exercidas.iter().map(|(n, _)| n.to_string()).collect();
    nomes_exercidos.sort();

    assert_eq!(
        do_idl, nomes_exercidos,
        "T89: o IDL e a lista da suite divergiram.\n  no IDL: {do_idl:?}\n  \
         exercidas: {nomes_exercidos:?}\nInstrucao privilegiada nova entra na \
         lista deste teste JUNTO com o teste de recusa dela — nunca depois."
    );

    // São **quatorze** com autoridade signatária: treze amarradas por `has_one`
    // e a gênese. **É o regime permanente** — a `migrate_vault_resgate` saiu no
    // Upgrade B, e com ela o programa ficou sem nenhuma instrução privilegiada
    // que reescreva bytes crus de conta viva. É esse binário que a firma audita.
    //
    // O lote pré-auditoria mexeu nos dois lados: saiu `process_redemptions`
    // (a fila D+30 inteira), saiu `migrate_vault_lucro`, entraram
    // `efetivar_resgate_capital`, `update_endereco_resgate`, `set_nav_oracle` e
    // `migrate_vault_resgate`. O `solicitar_resgate_capital` é do cotista e não
    // conta; `carimbar_nav` e `marcar_vencido` são **permissionless** e também
    // não — elas não têm signatário nenhum, que é o ponto delas.
    //
    // Não são "cinco", como o dossiê da F1 dizia, nem "sete", como o mandato da
    // F2 herdou dele: a lista sai do IDL para não voltar a envelhecer.
    //
    // O Upgrade E somou `ajustar_parametro`, que e' permanente e existe porque
    // valor deixou de morar no binario (D-F2-21). A `migrar_vault_parametros`
    // que veio junto era TEMPORARIA e **saiu no Upgrade F** — a linha dela nesta
    // lista saiu na mesma sessao, e e' por isso que a contagem voltou a quinze e
    // nao ficou em dezesseis com um item morto.
    //
    // O Upgrade F tirou a `migrar_vault_parametros` e trouxe a
    // `corrigir_deployed_usdc`, que tambem era TEMPORARIA. **O Upgrade G a
    // removeu**, e a contagem voltou a quinze — como estava previsto na linha
    // acima quando ela entrou.
    //
    // O G tambem trouxe a `deposit_para`, que NAO entra nesta conta: ela nao
    // tem `authority`, e quem paga assina por si. Conferir isto e' o servico
    // deste teste — a lista sai do IDL, nao de memoria.
    //
    // **O Upgrade H levou a quinze para DEZESSEIS**, com a `set_whitelist_operator`
    // (`D-F2-34`). Ela e' permanente, e nao temporaria como as duas acima: nomear
    // e destituir o porteiro e' ato de mesa que vai existir enquanto houver
    // whitelist.
    //
    // ⚠️ E a `update_whitelist` CONTINUA nesta lista mesmo tendo ganhado o
    // caminho do porteiro. Ela segue aceitando a autoridade, e o porteiro e' uma
    // porta a mais — nao a substituicao da caneta da mesa.
    //
    // **O Upgrade J levou a DEZOITO** (D-F2-43) e o **K trouxe a DEZESSETE**
    // (D-F2-45): entrou `ajustar_deployed_usdc` (permanente — a ferramenta de
    // correcao do custo de capital) e sairam as duas temporarias da cerimonia
    // do J, `migrar_vault_indice` e `atualizar_extra_account_meta_list`. O
    // `deposit_especial` continua FORA da lista: a caneta dele e' a dona da
    // gaveta, nao a autoridade.
    assert_eq!(
        do_idl.len(),
        17,
        "contagem de privilegiadas mudou: {do_idl:?}"
    );

    // E `publish_nav` NÃO entra: a conta signatária dela chama `publisher`, e o
    // caminho do oráculo não é privilegiado. Ela tem um **ramo** privilegiado
    // (D-F2-06), que é outra coisa e está provado no T75.
    assert!(
        !do_idl.iter().any(|n| n == "publish_nav"),
        "publish_nav nao e privilegiada — tem ramo privilegiado, ver D-F2-06"
    );
}
