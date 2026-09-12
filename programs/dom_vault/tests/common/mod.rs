//! Ambiente compartilhado pelos testes de integração.
//!
//! Monta o DOM como ele vai existir em devnet: mint Token-2022 com hook,
//! autoridade de mint no PDA do cofre (D15), USDC clássico, cofre inicializado e
//! conta de validação da interface gravada. Daí para frente os testes só usam
//! instruções de verdade — não existe atalho para criar cota.

// Cada binário de teste usa um pedaço deste módulo; o resto vira "dead code"
// aos olhos do compilador. É esperado.
#![allow(dead_code)]
// O `TransactionResult` da litesvm carrega os logs inteiros na variante de erro
// — e é justamente deles que o teste precisa quando fica vermelho.
#![allow(clippy::result_large_err)]

use {
    anchor_lang::{
        prelude::Pubkey,
        solana_program::{
            clock::Clock,
            instruction::{error::InstructionError, AccountMeta, Instruction},
            program_pack::Pack,
            system_instruction,
        },
        AccountDeserialize, Discriminator, InstructionData, ToAccountMetas,
    },
    base64::{engine::general_purpose::STANDARD as BASE64, Engine},
    dom_vault::{
        constants::{
            ESCROW_DOM_SEED, ESCROW_SEED, FEE_SHARE_SEED, LUCRO_SEED, NAV_GENESIS, RESGATE_SEED,
            TREASURY_SEED, VAULT_SEED, WHITELIST_SEED,
        },
        error::DomError,
        state::{FeeShareLedger, LucroSacado, PedidoResgate, Vault},
    },
    litesvm::{
        types::{TransactionMetadata, TransactionResult},
        LiteSVM,
    },
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
    solana_transaction_error::TransactionError,
    spl_token_2022_interface::{
        extension::{transfer_hook, ExtensionType, StateWithExtensions},
        instruction::{burn, initialize_account3, initialize_mint2, mint_to, transfer_checked},
        state::{Account as TokenAccountState, Mint as MintState},
    },
    spl_transfer_hook_interface::get_extra_account_metas_address,
};

pub const DECIMALS: u8 = 6;
pub const SOL: u64 = 1_000_000_000;
/// Uma unidade inteira de USDC ou de cota — as duas têm 6 casas (D9).
pub const UNIT: u64 = 1_000_000;

pub fn program_bytes() -> &'static [u8] {
    include_bytes!(concat!(
        env!("CARGO_TARGET_TMPDIR"),
        "/../deploy/dom_vault.so"
    ))
}

pub fn token_2022() -> Pubkey {
    spl_token_2022_interface::ID
}

/// SPL Token clássico — é onde vive o USDC de devnet.
pub fn token_classic() -> Pubkey {
    spl_token_2022_interface::inline_spl_token::ID
}

pub fn vault_pda() -> Pubkey {
    Pubkey::find_program_address(&[VAULT_SEED], &dom_vault::ID).0
}

pub fn escrow_pda() -> Pubkey {
    Pubkey::find_program_address(&[ESCROW_SEED], &dom_vault::ID).0
}

pub fn escrow_dom_pda() -> Pubkey {
    Pubkey::find_program_address(&[ESCROW_DOM_SEED], &dom_vault::ID).0
}

pub fn fee_share_pda(socio: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[FEE_SHARE_SEED, socio.as_ref()], &dom_vault::ID).0
}

/// Marca de saque de lucro por carteira (D-F2-09).
pub fn lucro_pda(cotista: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[LUCRO_SEED, cotista.as_ref()], &dom_vault::ID).0
}

/// A conta da fila aposentada. Não é mais criada pelo `initialize`; existe só
/// porque a migração precisa lê-la para recusar migrar com pedido pendente.
pub fn queue_pda() -> Pubkey {
    Pubkey::find_program_address(&[b"queue"], &dom_vault::ID).0
}

/// PDA de um pedido de resgate de capital.
pub fn pedido_resgate_pda(id: u64) -> Pubkey {
    Pubkey::find_program_address(&[RESGATE_SEED, &id.to_le_bytes()], &dom_vault::ID).0
}

pub fn treasury_pda() -> Pubkey {
    Pubkey::find_program_address(&[TREASURY_SEED], &dom_vault::ID).0
}

pub fn whitelist_pda(wallet: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[WHITELIST_SEED, wallet.as_ref()], &dom_vault::ID).0
}

pub fn validation_pda(mint: &Pubkey) -> Pubkey {
    get_extra_account_metas_address(mint, &dom_vault::ID)
}

// ---------------------------------------------------------------------------
// Envio e asserções
// ---------------------------------------------------------------------------

pub fn send(
    svm: &mut LiteSVM,
    payer: &Keypair,
    ixs: &[Instruction],
    extra_signers: &[&Keypair],
) -> TransactionResult {
    svm.expire_blockhash();
    let blockhash = svm.latest_blockhash();
    let message = Message::new_with_blockhash(ixs, Some(&payer.pubkey()), &blockhash);
    let mut signers: Vec<&Keypair> = vec![payer];
    signers.extend_from_slice(extra_signers);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message), &signers).unwrap();
    svm.send_transaction(tx)
}

/// Confere o código custom exato, com os logs no `panic` quando não bate — sem
/// isso um teste vermelho não diz nada.
pub fn assert_dom_error(result: TransactionResult, expected: DomError, cenario: &str) {
    let expected_code = u32::from(expected);
    match result {
        Ok(meta) => panic!(
            "{cenario}: transacao PASSOU, esperado erro {expected_code}\n{}",
            meta.pretty_logs()
        ),
        Err(failed) => match failed.err {
            TransactionError::InstructionError(_, InstructionError::Custom(code)) => {
                assert_eq!(
                    code,
                    expected_code,
                    "{cenario}: esperado {expected_code}, veio {code}\n{}",
                    failed.meta.pretty_logs()
                );
            }
            other => panic!(
                "{cenario}: esperado Custom({expected_code}), veio {other:?}\n{}",
                failed.meta.pretty_logs()
            ),
        },
    }
}

/// Espelho de `dom_vault::math::nav_dentro_do_bound` para o harness decidir o
/// caminho de publicação sem depender de item privado do programa.
pub fn dentro_do_bound(anterior: u64, novo: u64) -> bool {
    let delta = (anterior.max(novo) - anterior.min(novo)) as u128;
    delta.saturating_mul(dom_vault::NAV_BOUND_DEN)
        <= (anterior as u128).saturating_mul(dom_vault::NAV_BOUND_NUM)
}

pub fn assert_ok(result: TransactionResult, cenario: &str) -> TransactionMetadata {
    match result {
        Ok(meta) => meta,
        Err(failed) => panic!(
            "{cenario}: deveria passar, falhou com {:?}\n{}",
            failed.err,
            failed.meta.pretty_logs()
        ),
    }
}

/// Procura um evento do Anchor nos logs pelo discriminador — `emit!` publica em
/// `Program data:` codificado em base64.
pub fn tem_evento<E: Discriminator>(meta: &TransactionMetadata) -> bool {
    meta.logs
        .iter()
        .filter_map(|linha| linha.strip_prefix("Program data: "))
        .filter_map(|dados| BASE64.decode(dados).ok())
        .any(|bytes| bytes.starts_with(E::DISCRIMINATOR))
}

// ---------------------------------------------------------------------------
// Mints e contas de token
// ---------------------------------------------------------------------------

/// Mint Token-2022 com a extensão `TransferHook`. Tudo parametrizado porque os
/// testes do hook precisam montar mints hostis.
pub fn create_mint_com_hook(
    svm: &mut LiteSVM,
    payer: &Keypair,
    mint: &Keypair,
    mint_authority: &Pubkey,
    hook_authority: Option<Pubkey>,
    hook_program: Option<Pubkey>,
) {
    let space =
        ExtensionType::try_calculate_account_len::<MintState>(&[ExtensionType::TransferHook])
            .unwrap();
    let lamports = svm.minimum_balance_for_rent_exemption(space);

    let ixs = [
        system_instruction::create_account(
            &payer.pubkey(),
            &mint.pubkey(),
            lamports,
            space as u64,
            &token_2022(),
        ),
        // A extensão precisa ser inicializada ANTES do mint.
        transfer_hook::instruction::initialize(
            &token_2022(),
            &mint.pubkey(),
            hook_authority,
            hook_program,
        )
        .unwrap(),
        initialize_mint2(
            &token_2022(),
            &mint.pubkey(),
            mint_authority,
            None,
            DECIMALS,
        )
        .unwrap(),
    ];
    assert_ok(send(svm, payer, &ixs, &[mint]), "create_mint_com_hook");
}

/// Mint clássico, sem extensão — o papel do USDC.
pub fn create_mint_classico(
    svm: &mut LiteSVM,
    payer: &Keypair,
    mint: &Keypair,
    mint_authority: &Pubkey,
) {
    let space = MintState::LEN;
    let lamports = svm.minimum_balance_for_rent_exemption(space);

    let ixs = [
        system_instruction::create_account(
            &payer.pubkey(),
            &mint.pubkey(),
            lamports,
            space as u64,
            &token_classic(),
        ),
        initialize_mint2(
            &token_classic(),
            &mint.pubkey(),
            mint_authority,
            None,
            DECIMALS,
        )
        .unwrap(),
    ];
    assert_ok(send(svm, payer, &ixs, &[mint]), "create_mint_classico");
}

/// Conta de token do DOM: precisa de espaço para a extensão
/// `TransferHookAccount`, senão o Token-2022 recusa a conta.
pub fn create_conta_dom(
    svm: &mut LiteSVM,
    payer: &Keypair,
    mint: &Pubkey,
    owner: &Pubkey,
) -> Pubkey {
    let space = ExtensionType::try_calculate_account_len::<TokenAccountState>(&[
        ExtensionType::TransferHookAccount,
    ])
    .unwrap();
    create_conta_token(svm, payer, mint, owner, &token_2022(), space)
}

pub fn create_conta_usdc(
    svm: &mut LiteSVM,
    payer: &Keypair,
    mint: &Pubkey,
    owner: &Pubkey,
) -> Pubkey {
    create_conta_token(
        svm,
        payer,
        mint,
        owner,
        &token_classic(),
        TokenAccountState::LEN,
    )
}

fn create_conta_token(
    svm: &mut LiteSVM,
    payer: &Keypair,
    mint: &Pubkey,
    owner: &Pubkey,
    token_program: &Pubkey,
    space: usize,
) -> Pubkey {
    let account = Keypair::new();
    let lamports = svm.minimum_balance_for_rent_exemption(space);

    let ixs = [
        system_instruction::create_account(
            &payer.pubkey(),
            &account.pubkey(),
            lamports,
            space as u64,
            token_program,
        ),
        initialize_account3(token_program, &account.pubkey(), mint, owner).unwrap(),
    ];
    assert_ok(send(svm, payer, &ixs, &[&account]), "create_conta_token");
    account.pubkey()
}

pub fn mint_tokens(
    svm: &mut LiteSVM,
    payer: &Keypair,
    token_program: &Pubkey,
    mint: &Pubkey,
    mint_authority: &Keypair,
    destination: &Pubkey,
    amount: u64,
) {
    let ix = mint_to(
        token_program,
        mint,
        destination,
        &mint_authority.pubkey(),
        &[],
        amount,
    )
    .unwrap();
    assert_ok(send(svm, payer, &[ix], &[mint_authority]), "mint_to");
}

pub fn saldo(svm: &LiteSVM, conta: &Pubkey) -> u64 {
    let account = svm.get_account(conta).unwrap();
    StateWithExtensions::<TokenAccountState>::unpack(&account.data)
        .unwrap()
        .base
        .amount
}

pub fn supply(svm: &LiteSVM, mint: &Pubkey) -> u64 {
    let account = svm.get_account(mint).unwrap();
    StateWithExtensions::<MintState>::unpack(&account.data)
        .unwrap()
        .base
        .supply
}

// ---------------------------------------------------------------------------
// Ambiente do DOM
// ---------------------------------------------------------------------------

pub struct Holder {
    pub wallet: Keypair,
    pub dom: Pubkey,
    pub usdc: Pubkey,
}

pub struct Env {
    pub svm: LiteSVM,
    pub payer: Keypair,
    /// Vault PDA do Squads em produção (D2); aqui, uma chave de teste.
    pub authority: Keypair,
    /// Publica NAV. Separado da autoridade de propósito (T35).
    pub oracle: Keypair,
    pub dom_mint: Pubkey,
    pub usdc_mint: Pubkey,
    pub usdc_authority: Keypair,
    /// As três carteiras de sócio, já com contas de DOM e de USDC.
    pub socios: Vec<Holder>,
}

/// Relógio de partida dos testes. Fixo para que D+30 e ciclo sejam
/// determinísticos — nada aqui depende da hora em que a suite roda.
pub const T0: i64 = 1_700_000_000;

impl Env {
    /// Cofre inicializado **e com NAV publicado** — o estado em que a produção
    /// aceita o primeiro aporte.
    pub fn new() -> Self {
        Self::montar(true)
    }

    /// Cofre inicializado e **sem NAV publicado** — gênese crua. Existe para
    /// exercer a recusa do `deposit` nesse canto (decisão da mesa, 2026-08-20).
    pub fn sem_nav_publicado() -> Self {
        Self::montar(false)
    }

    fn montar(publicar_nav: bool) -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program(dom_vault::ID, program_bytes()).unwrap();

        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 1_000 * SOL).unwrap();

        let mut clock: Clock = svm.get_sysvar();
        clock.unix_timestamp = T0;
        svm.set_sysvar(&clock);

        let authority = Keypair::new();
        let oracle = Keypair::new();
        let usdc_authority = Keypair::new();
        let socios: Vec<Keypair> = (0..3).map(|_| Keypair::new()).collect();

        let usdc_mint = Keypair::new();
        create_mint_classico(&mut svm, &payer, &usdc_mint, &usdc_authority.pubkey());

        // Autoridade de mint do DOM = PDA do cofre (D15). Sem isso o
        // `initialize` recusa o mint.
        let dom_mint = Keypair::new();
        create_mint_com_hook(
            &mut svm,
            &payer,
            &dom_mint,
            &vault_pda(),
            Some(authority.pubkey()),
            Some(dom_vault::ID),
        );

        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::Initialize {
                nav_oracle: oracle.pubkey(),
                socios: [socios[0].pubkey(), socios[1].pubkey(), socios[2].pubkey()],
            }
            .data(),
            dom_vault::accounts::Initialize {
                payer: payer.pubkey(),
                authority: authority.pubkey(),
                dom_mint: dom_mint.pubkey(),
                usdc_mint: usdc_mint.pubkey(),
                vault: vault_pda(),
                treasury: treasury_pda(),
                escrow_authority: escrow_pda(),
                escrow_dom: escrow_dom_pda(),
                dom_token_program: token_2022(),
                usdc_token_program: token_classic(),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
        );
        assert_ok(send(&mut svm, &payer, &[ix], &[&authority]), "initialize");

        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::InitializeExtraAccountMetaList {}.data(),
            dom_vault::accounts::InitializeExtraAccountMetaList {
                payer: payer.pubkey(),
                authority: authority.pubkey(),
                vault: vault_pda(),
                mint: dom_mint.pubkey(),
                extra_account_meta_list: validation_pda(&dom_mint.pubkey()),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
        );
        assert_ok(
            send(&mut svm, &payer, &[ix], &[&authority]),
            "initialize_extra_account_meta_list",
        );

        let mut env = Self {
            svm,
            payer,
            authority,
            oracle,
            dom_mint: dom_mint.pubkey(),
            usdc_mint: usdc_mint.pubkey(),
            usdc_authority,
            socios: Vec::new(),
        };

        // Contas dos sócios existem desde o começo: `deposit_especial` minta
        // direto nelas e não cria conta de token.
        env.socios = socios
            .into_iter()
            .map(|wallet| env.carteira_de(wallet, 0))
            .collect();

        // Primeira publicação de NAV, no valor de gênese. Sem ela o `deposit`
        // recusa com `NavNeverPublished` — decisão da mesa de 2026-08-20, que
        // fechou o canto de gênese do DV11. O ambiente de teste faz o que a
        // produção faz: o oráculo publica **antes** do primeiro aporte.
        //
        // Quem precisa do cofre ainda sem NAV usa `Env::sem_nav_publicado`.
        if publicar_nav {
            env.publish_nav(NAV_GENESIS);
        }
        env
    }

    /// Sobe um ambiente cru e chama `initialize` com os sócios dados, sem
    /// assertar sucesso. É o que permite testar a rejeição do T57, já que o
    /// `Env::new` normal já entrega o cofre inicializado.
    pub fn tentar_initialize(socios: [Pubkey; 3]) -> TransactionResult {
        let mut svm = LiteSVM::new();
        svm.add_program(dom_vault::ID, program_bytes()).unwrap();

        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 1_000 * SOL).unwrap();
        let mut clock: Clock = svm.get_sysvar();
        clock.unix_timestamp = T0;
        svm.set_sysvar(&clock);

        let authority = Keypair::new();
        let oracle = Keypair::new();
        let usdc_mint = Keypair::new();
        create_mint_classico(&mut svm, &payer, &usdc_mint, &authority.pubkey());
        let dom_mint = Keypair::new();
        create_mint_com_hook(
            &mut svm,
            &payer,
            &dom_mint,
            &vault_pda(),
            Some(authority.pubkey()),
            Some(dom_vault::ID),
        );

        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::Initialize {
                nav_oracle: oracle.pubkey(),
                socios,
            }
            .data(),
            dom_vault::accounts::Initialize {
                payer: payer.pubkey(),
                authority: authority.pubkey(),
                dom_mint: dom_mint.pubkey(),
                usdc_mint: usdc_mint.pubkey(),
                vault: vault_pda(),
                treasury: treasury_pda(),
                escrow_authority: escrow_pda(),
                escrow_dom: escrow_dom_pda(),
                dom_token_program: token_2022(),
                usdc_token_program: token_classic(),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
        );
        send(&mut svm, &payer, &[ix], &[&authority])
    }

    pub fn ledger(&self, socio: &Pubkey) -> FeeShareLedger {
        let account = self.svm.get_account(&fee_share_pda(socio)).unwrap();
        FeeShareLedger::try_deserialize(&mut &account.data[..]).unwrap()
    }

    /// O livro de `fee_share` deste sócio ainda nem existe? É o estado de quem
    /// nunca recebeu distribuição — a conta é criada preguiçosamente.
    pub fn ledger_ausente(&self, socio: &Pubkey) -> bool {
        self.svm
            .get_account(&fee_share_pda(socio))
            .is_none_or(|c| c.data.is_empty())
    }

    /// Marca de saque de lucro desta carteira, se já existir (D-F2-09).
    pub fn marca_de_lucro(&self, cotista: &Pubkey) -> Option<LucroSacado> {
        let conta = self.svm.get_account(&lucro_pda(cotista))?;
        if conta.data.is_empty() {
            return None;
        }
        Some(LucroSacado::try_deserialize(&mut &conta.data[..]).unwrap())
    }

    /// Fecha a janela de saque de lucro do jeito que a produção fecha: mandando
    /// capital a campo (D-F2-09). Registra um destino operacional se ainda não
    /// houver nenhum e devolve o valor imediatamente, para não deixar o caixa
    /// do teste diferente do que estava.
    pub fn fechar_janela_de_lucro(&mut self) {
        assert!(
            self.vault().lucro_sacavel_restante > 0,
            "fechar_janela_de_lucro chamado com a janela ja' fechada"
        );
        let ops = self.carteira_de(Keypair::new(), 0);
        self.update_deploy_allowlist(0, ops.wallet.pubkey());
        self.deploy_capital(ops.usdc, 1);
        self.return_capital(&ops, 1);
        self.update_deploy_allowlist(0, Pubkey::default());
        assert_eq!(self.vault().lucro_sacavel_restante, 0, "janela fechou");
    }

    /// O pedido de resgate de `id`. Falha se não existir.
    pub fn pedido(&self, id: u64) -> PedidoResgate {
        let account = self
            .svm
            .get_account(&pedido_resgate_pda(id))
            .unwrap_or_else(|| panic!("pedido {id} nao existe"));
        PedidoResgate::try_deserialize(&mut &account.data[..]).unwrap()
    }

    /// `Some` se o pedido existe; `None` se nunca foi aberto.
    pub fn pedido_opt(&self, id: u64) -> Option<PedidoResgate> {
        self.svm
            .get_account(&pedido_resgate_pda(id))
            .map(|c| PedidoResgate::try_deserialize(&mut &c.data[..]).unwrap())
    }

    /// Saldo de cotas parado no escrow. A invariante do fuzz compara com o
    /// `cotas_travadas_resgate` do cofre.
    pub fn escrow_cotas(&self) -> u64 {
        let conta = self.svm.get_account(&escrow_dom_pda()).unwrap();
        u64::from_le_bytes(conta.data[64..72].try_into().unwrap())
    }

    pub fn agora(&self) -> i64 {
        let clock: Clock = self.svm.get_sysvar();
        clock.unix_timestamp
    }

    /// Move o relógio da rede **com o fundo operando**: se o salto deixaria o
    /// NAV velho (DV11), o oráculo republica o mesmo valor no fim, que é o que
    /// um backend vivo faria ao longo desses dias.
    ///
    /// É o comportamento certo para a maioria dos testes: eles saltam 30 dias
    /// para exercer D+30 e ciclo, não para simular oráculo caído. Quem quer o
    /// oráculo caído usa `avancar_sem_oraculo`.
    pub fn avancar(&mut self, segundos: i64) {
        self.avancar_sem_oraculo(segundos);
        let v = self.vault();
        // `nav_ts == 0` é gênese: não há publicação para renovar.
        if v.nav_ts != 0 && self.agora().saturating_sub(v.nav_ts) > dom_vault::MAX_NAV_STALENESS {
            // **Sem mexer no relógio.** `publish_nav` anda um segundo, e esse
            // segundo furaria os testes de borda: `avancar(REDEEM_DELAY - 1)`
            // viraria D+30 exato e o pedido passaria a ser pago um segundo antes
            // da hora. Aqui o relógio já andou o que tinha de andar; só falta a
            // publicação, no instante corrente.
            let agora = self.agora();
            let oracle = self.oracle.insecure_clone();
            assert_ok(
                self.publish_nav_raw(&oracle, v.nav, agora),
                "renovacao do NAV durante avancar",
            );
            // O salto que tornou o NAV velho e' sempre maior que o intervalo
            // minimo (26h contra 1h), entao a renovacao nunca esbarra nele.
        }
    }

    /// Move o relógio **sem ninguém publicar NAV**. É o backend caído.
    pub fn avancar_sem_oraculo(&mut self, segundos: i64) {
        let mut clock: Clock = self.svm.get_sysvar();
        clock.unix_timestamp += segundos;
        self.svm.set_sysvar(&clock);
    }

    pub fn vault(&self) -> Vault {
        let account = self.svm.get_account(&vault_pda()).unwrap();
        Vault::try_deserialize(&mut &account.data[..]).unwrap()
    }

    pub fn update_whitelist_raw(
        &mut self,
        signer: &Keypair,
        wallet: &Pubkey,
        active: bool,
    ) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::UpdateWhitelist {
                owner: *wallet,
                active,
                // Sem excecao: vale o piso do cofre. Os testes que exercitam o
                // piso proprio usam `update_whitelist_com_piso`.
                min_deposit_proprio: 0,
            }
            .data(),
            dom_vault::accounts::UpdateWhitelist {
                payer: self.payer.pubkey(),
                authority: signer.pubkey(),
                vault: vault_pda(),
                entry: whitelist_pda(wallet),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    /// Grava a entrada com um piso SO' DESTA CARTEIRA. `0` = usa o do cofre.
    pub fn update_whitelist_com_piso(
        &mut self,
        wallet: &Pubkey,
        active: bool,
        piso: u64,
    ) -> TransactionResult {
        let authority = self.authority.insecure_clone();
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::UpdateWhitelist {
                owner: *wallet,
                active,
                min_deposit_proprio: piso,
            }
            .data(),
            dom_vault::accounts::UpdateWhitelist {
                payer: self.payer.pubkey(),
                authority: authority.pubkey(),
                vault: vault_pda(),
                entry: whitelist_pda(wallet),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
        );
        send(&mut self.svm, &self.payer, &[ix], &[&authority])
    }

    /// **Planta uma entrada no LAYOUT ANTIGO, de 42 bytes.**
    ///
    /// Existe para provar que o Upgrade G nao quebra quem ja' estava aprovado —
    /// e essa prova nao se faz com o layout novo. Cria pela porta normal e
    /// ENCOLHE a conta para o tamanho que ela tinha antes do campo entrar.
    /// Aporte EM NOME DE OUTRA CARTEIRA. Quem paga assina; quem recebe, nao.
    pub fn deposit_para_raw(
        &mut self,
        aportador: &Holder,
        beneficiario: &Holder,
        usdc_amount: u64,
    ) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::DepositPara { usdc_amount }.data(),
            dom_vault::accounts::DepositPara {
                aportador: aportador.wallet.pubkey(),
                aportador_whitelist: whitelist_pda(&aportador.wallet.pubkey()),
                beneficiario: beneficiario.wallet.pubkey(),
                beneficiario_whitelist: whitelist_pda(&beneficiario.wallet.pubkey()),
                vault: vault_pda(),
                dom_mint: self.dom_mint,
                usdc_mint: self.usdc_mint,
                aportador_usdc: aportador.usdc,
                treasury: treasury_pda(),
                beneficiario_dom: beneficiario.dom,
                dom_token_program: token_2022(),
                usdc_token_program: anchor_spl::token::ID,
            }
            .to_account_metas(None),
        );
        let a = aportador.wallet.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&a])
    }

    pub fn plantar_whitelist_antiga(&mut self, wallet: &Pubkey, active: bool) {
        self.whitelist(wallet, active);
        let pda = whitelist_pda(wallet);
        let mut conta = self.svm.get_account(&pda).unwrap();
        conta.data.truncate(42);
        assert_eq!(conta.data.len(), 42, "o layout antigo tem 42 bytes");
        self.svm.set_account(pda, conta).unwrap();
    }

    pub fn whitelist(&mut self, wallet: &Pubkey, active: bool) -> TransactionMetadata {
        let authority = self.authority.insecure_clone();
        assert_ok(
            self.update_whitelist_raw(&authority, wallet, active),
            "update_whitelist",
        )
    }

    pub fn enable_cap_raw(&mut self, signer: &Keypair) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::EnableCap {}.data(),
            dom_vault::accounts::EnableCap {
                authority: signer.pubkey(),
                vault: vault_pda(),
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    pub fn enable_cap(&mut self) -> TransactionMetadata {
        let authority = self.authority.insecure_clone();
        assert_ok(self.enable_cap_raw(&authority), "enable_cap")
    }

    pub fn deposit_raw(&mut self, holder: &Holder, usdc_amount: u64) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::Deposit { usdc_amount }.data(),
            dom_vault::accounts::Deposit {
                depositor: holder.wallet.pubkey(),
                vault: vault_pda(),
                depositor_whitelist: whitelist_pda(&holder.wallet.pubkey()),
                dom_mint: self.dom_mint,
                usdc_mint: self.usdc_mint,
                depositor_usdc: holder.usdc,
                treasury: treasury_pda(),
                depositor_dom: holder.dom,
                dom_token_program: token_2022(),
                usdc_token_program: token_classic(),
            }
            .to_account_metas(None),
        );
        let wallet = holder.wallet.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&wallet])
    }

    pub fn deposit(&mut self, holder: &Holder, usdc_amount: u64) -> TransactionMetadata {
        assert_ok(self.deposit_raw(holder, usdc_amount), "deposit")
    }

    /// Carteira com contas de DOM e de USDC, USDC no bolso e whitelist ativa.
    /// **Sem cota** — cota só nasce de `deposit`.
    pub fn carteira(&mut self, usdc_no_bolso: u64) -> Holder {
        self.carteira_de(Keypair::new(), usdc_no_bolso)
    }

    /// Carteira criada e financiada, mas **NAO aprovada** pela mesa. Existe para
    /// provar as recusas — e sem ela a trava do `deposit_para` nao teria ensaio.
    pub fn carteira_sem_whitelist(&mut self, usdc_no_bolso: u64) -> Holder {
        let wallet = Keypair::new();
        self.svm.airdrop(&wallet.pubkey(), SOL).unwrap();
        let dom = create_conta_dom(&mut self.svm, &self.payer, &self.dom_mint, &wallet.pubkey());
        let usdc = create_conta_usdc(
            &mut self.svm,
            &self.payer,
            &self.usdc_mint,
            &wallet.pubkey(),
        );
        if usdc_no_bolso > 0 {
            let usdc_mint = self.usdc_mint;
            let usdc_authority = self.usdc_authority.insecure_clone();
            mint_tokens(
                &mut self.svm,
                &self.payer,
                &token_classic(),
                &usdc_mint,
                &usdc_authority,
                &usdc,
                usdc_no_bolso,
            );
        }
        // **Sem `self.whitelist(...)`** — e' o que a distingue da `carteira_de`.
        Holder { wallet, dom, usdc }
    }

    pub fn carteira_de(&mut self, wallet: Keypair, usdc_no_bolso: u64) -> Holder {
        self.svm.airdrop(&wallet.pubkey(), SOL).unwrap();

        let dom = create_conta_dom(&mut self.svm, &self.payer, &self.dom_mint, &wallet.pubkey());
        let usdc = create_conta_usdc(
            &mut self.svm,
            &self.payer,
            &self.usdc_mint,
            &wallet.pubkey(),
        );

        if usdc_no_bolso > 0 {
            let usdc_mint = self.usdc_mint;
            let usdc_authority = self.usdc_authority.insecure_clone();
            mint_tokens(
                &mut self.svm,
                &self.payer,
                &token_classic(),
                &usdc_mint,
                &usdc_authority,
                &usdc,
                usdc_no_bolso,
            );
        }

        self.whitelist(&wallet.pubkey(), true);
        Holder { wallet, dom, usdc }
    }

    /// Carteira habilitada que já aportou — o caminho normal de quem tem cota.
    pub fn cotista(&mut self, usdc_aportado: u64) -> Holder {
        let holder = self.carteira(usdc_aportado);
        self.deposit(&holder, usdc_aportado);
        holder
    }

    /// Transferência real, com as contas extras na ordem que o Token-2022
    /// espera: extras da conta de validação, depois o program id do hook,
    /// depois a própria conta de validação.
    pub fn transfer(
        &mut self,
        origem: &Holder,
        destino_conta: &Pubkey,
        destino_owner: &Pubkey,
        amount: u64,
    ) -> TransactionResult {
        let mut ix = transfer_checked(
            &token_2022(),
            &origem.dom,
            &self.dom_mint,
            destino_conta,
            &origem.wallet.pubkey(),
            &[],
            amount,
            DECIMALS,
        )
        .unwrap();
        ix.accounts.extend_from_slice(&[
            AccountMeta::new_readonly(vault_pda(), false),
            AccountMeta::new_readonly(whitelist_pda(&origem.wallet.pubkey()), false),
            AccountMeta::new_readonly(whitelist_pda(destino_owner), false),
            AccountMeta::new_readonly(dom_vault::ID, false),
            AccountMeta::new_readonly(validation_pda(&self.dom_mint), false),
        ]);
        let wallet = origem.wallet.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&wallet])
    }

    /// Queima de cota pelo próprio dono. Não passa pelo hook (só transferência
    /// passa) e serve para reduzir supply sem `process_redemptions`.
    pub fn burn(&mut self, holder: &Holder, amount: u64) -> TransactionResult {
        let ix = burn(
            &token_2022(),
            &holder.dom,
            &self.dom_mint,
            &holder.wallet.pubkey(),
            &[],
            amount,
        )
        .unwrap();
        let wallet = holder.wallet.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&wallet])
    }

    // -----------------------------------------------------------------------
    // Bloco 4: NAV, pedido e processamento
    // -----------------------------------------------------------------------

    pub fn publish_nav_raw(
        &mut self,
        signer: &Keypair,
        nav: u64,
        timestamp: i64,
    ) -> TransactionResult {
        self.publish_nav_raw_com_hash(signer, nav, [7u8; 32], timestamp)
    }

    /// Igual, com o laudo escolhido pelo teste — o que o T76 usa para provar
    /// que a atestação vai para o **estado** (DV12), não só para o evento.
    pub fn publish_nav_raw_com_hash(
        &mut self,
        signer: &Keypair,
        nav: u64,
        attestation_hash: [u8; 32],
        timestamp: i64,
    ) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::PublishNav {
                nav,
                attestation_hash,
                timestamp,
            }
            .data(),
            dom_vault::accounts::PublishNav {
                publisher: signer.pubkey(),
                vault: vault_pda(),
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    /// Publica NAV carimbado com o relógio corrente — o caminho normal.
    ///
    /// Anda um segundo antes: o timestamp do NAV é monotônico estrito (T36), e
    /// duas publicações no mesmo segundo não são um cenário real.
    ///
    /// **Roteia como a produção rotearia (DV11).** Variação dentro do limite de
    /// 15% vai pelo oráculo; acima dele, o oráculo é recusado e quem publica é a
    /// autoridade, por proposta — a válvula. O helper escolhe o mesmo caminho
    /// que a mesa escolheria, para o teste continuar dizendo "o NAV passou a ser
    /// X" sem mentir sobre como isso acontece.
    ///
    /// Quem quer exercer a recusa usa `publish_nav_raw` com o oráculo.
    /// Publica NAV pelo caminho certo: oráculo se couber no bound, válvula da
    /// mesa se não couber.
    ///
    /// **Anda o relógio até o intervalo mínimo do oráculo.** O
    /// `MIN_NAV_PUBLISH_INTERVAL` é o que transforma os 15% por publicação em
    /// limite de taxa, e sem esse avanço todo teste que publica duas vezes levaria
    /// `NavPublicacaoMuitoCedo` por um motivo que não tem nada a ver com o que ele
    /// está provando. Quem testa a trava usa `publish_nav_raw`, que não anda nada.
    ///
    /// A válvula da mesa não tem intervalo, então o avanço só vale para o oráculo.
    pub fn publish_nav(&mut self, nav: u64) -> TransactionMetadata {
        let anterior = self.vault().nav;
        let pelo_oraculo = dentro_do_bound(anterior, nav);

        if pelo_oraculo {
            let nav_ts = self.vault().nav_ts;
            let falta = if nav_ts == 0 {
                1
            } else {
                (nav_ts + dom_vault::constants::MIN_NAV_PUBLISH_INTERVAL - self.agora()).max(1)
            };
            self.avancar_sem_oraculo(falta);
        } else {
            self.avancar_sem_oraculo(1);
        }

        let agora = self.agora();
        let signer = if pelo_oraculo {
            self.oracle.insecure_clone()
        } else {
            self.authority.insecure_clone()
        };
        assert_ok(self.publish_nav_raw(&signer, nav, agora), "publish_nav")
    }

    /// Publica pela **válvula**, sempre — mesmo dentro do limite. Para os testes
    /// que afirmam o caminho da mesa, não o do oráculo.
    pub fn publish_nav_pela_mesa(&mut self, nav: u64) -> TransactionMetadata {
        self.avancar_sem_oraculo(1);
        let agora = self.agora();
        let mesa = self.authority.insecure_clone();
        assert_ok(
            self.publish_nav_raw(&mesa, nav, agora),
            "publish_nav pela mesa",
        )
    }

    /// Publica pelo **oráculo**, sempre — o caminho que o bound vigia.
    /// Publica pelo oráculo, sempre. Anda o relógio até o intervalo mínimo para
    /// que a recusa que o teste espera seja a que ele está provando, e não o
    /// `NavPublicacaoMuitoCedo`. Quem testa o intervalo usa `publish_nav_raw`.
    pub fn publish_nav_pelo_oraculo_raw(&mut self, nav: u64) -> TransactionResult {
        let nav_ts = self.vault().nav_ts;
        let falta = if nav_ts == 0 {
            1
        } else {
            (nav_ts + dom_vault::constants::MIN_NAV_PUBLISH_INTERVAL - self.agora()).max(1)
        };
        self.avancar_sem_oraculo(falta);
        let agora = self.agora();
        let oracle = self.oracle.insecure_clone();
        self.publish_nav_raw(&oracle, nav, agora)
    }

    // -----------------------------------------------------------------------
    // Resgate de capital
    // -----------------------------------------------------------------------

    /// Solicita resgate. As cotas saem da carteira para o escrow na própria
    /// instrução — não há transferência prévia como no `request_redeem` antigo,
    /// porque quem move a cota agora é o programa, com o cotista assinando.
    pub fn solicitar_resgate_raw(&mut self, holder: &Holder, cotas: u64) -> TransactionResult {
        let id = self.vault().proximo_pedido_id;

        // O resgate é COMPOSTO (D17): a transferência para o escrow vem primeiro,
        // montada pelo cliente com as contas extras do hook, e o pedido logo
        // atrás confere por introspecção. CPI daqui não resolveria o
        // `ExtraAccountMetaList`.
        let mut transferencia = transfer_checked(
            &token_2022(),
            &holder.dom,
            &self.dom_mint,
            &escrow_dom_pda(),
            &holder.wallet.pubkey(),
            &[],
            cotas,
            DECIMALS,
        )
        .unwrap();
        transferencia.accounts.extend_from_slice(&[
            AccountMeta::new_readonly(vault_pda(), false),
            AccountMeta::new_readonly(whitelist_pda(&holder.wallet.pubkey()), false),
            AccountMeta::new_readonly(whitelist_pda(&escrow_pda()), false),
            AccountMeta::new_readonly(dom_vault::ID, false),
            AccountMeta::new_readonly(validation_pda(&self.dom_mint), false),
        ]);

        let pedido = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::SolicitarResgateCapital { cotas }.data(),
            dom_vault::accounts::SolicitarResgateCapital {
                cotista: holder.wallet.pubkey(),
                vault: vault_pda(),
                cotista_whitelist: whitelist_pda(&holder.wallet.pubkey()),
                pedido: pedido_resgate_pda(id),
                dom_mint: self.dom_mint,
                cotista_dom: holder.dom,
                escrow_dom: escrow_dom_pda(),
                dom_token_program: token_2022(),
                instructions_sysvar: solana_instructions_sysvar::ID,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
        );

        let wallet = holder.wallet.insecure_clone();
        send(
            &mut self.svm,
            &self.payer,
            &[transferencia, pedido],
            &[&wallet],
        )
    }

    /// Devolve o `id` do pedido aberto.
    pub fn solicitar_resgate(&mut self, holder: &Holder, cotas: u64) -> u64 {
        let id = self.vault().proximo_pedido_id;
        assert_ok(
            self.solicitar_resgate_raw(holder, cotas),
            "solicitar_resgate",
        );
        id
    }

    /// Carimba o NAV corrente na catraca.
    ///
    /// **Não leva assinante nenhum** — a instrução não declara `Signer`. Quem
    /// paga a taxa é o payer da transação, e mais nada é exigido. É o que
    /// "permissionless" quer dizer aqui, e o motivo de ser seguro: quem chama não
    /// escolhe o número, e a catraca só desce.
    pub fn carimbar_nav_raw(&mut self, id: u64) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::CarimbarNav {}.data(),
            dom_vault::accounts::CarimbarNav {
                vault: vault_pda(),
                pedido: pedido_resgate_pda(id),
            }
            .to_account_metas(None),
        );
        send(&mut self.svm, &self.payer, &[ix], &[])
    }

    pub fn carimbar_nav(&mut self, id: u64) {
        assert_ok(self.carimbar_nav_raw(id), "carimbar_nav");
    }

    /// Marca vencido. Permissionless — sem assinante, como o `carimbar_nav`.
    pub fn marcar_vencido_raw(&mut self, id: u64) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::MarcarVencido {}.data(),
            dom_vault::accounts::MarcarVencido {
                vault: vault_pda(),
                pedido: pedido_resgate_pda(id),
            }
            .to_account_metas(None),
        );
        send(&mut self.svm, &self.payer, &[ix], &[])
    }

    pub fn marcar_vencido(&mut self, id: u64) {
        assert_ok(self.marcar_vencido_raw(id), "marcar_vencido");
    }

    /// Nomeia a conta pagadora dos resgates.
    pub fn update_endereco_resgate_raw(
        &mut self,
        signer: &Keypair,
        conta: Pubkey,
    ) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::UpdateEnderecoResgate {}.data(),
            dom_vault::accounts::UpdateEnderecoResgate {
                authority: signer.pubkey(),
                vault: vault_pda(),
                usdc_mint: self.usdc_mint,
                novo_endereco: conta,
                usdc_token_program: token_classic(),
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    pub fn update_endereco_resgate(&mut self, conta: Pubkey) {
        let authority = self.authority.insecure_clone();
        assert_ok(
            self.update_endereco_resgate_raw(&authority, conta),
            "update_endereco_resgate",
        );
    }

    /// Efetiva: queima a cota e paga do endereço de resgate.
    pub fn efetivar_resgate_raw(
        &mut self,
        signer: &Keypair,
        id: u64,
        pior_nav: u64,
        destino_usdc: Pubkey,
    ) -> TransactionResult {
        let endereco = self.vault().endereco_resgate;
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::EfetivarResgateCapital { pior_nav }.data(),
            dom_vault::accounts::EfetivarResgateCapital {
                authority: signer.pubkey(),
                vault: vault_pda(),
                pedido: pedido_resgate_pda(id),
                dom_mint: self.dom_mint,
                usdc_mint: self.usdc_mint,
                escrow_dom: escrow_dom_pda(),
                escrow_authority: escrow_pda(),
                endereco_resgate: endereco,
                cotista_usdc: destino_usdc,
                dom_token_program: token_2022(),
                usdc_token_program: token_classic(),
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    pub fn efetivar_resgate(&mut self, id: u64, pior_nav: u64, destino_usdc: Pubkey) {
        let authority = self.authority.insecure_clone();
        assert_ok(
            self.efetivar_resgate_raw(&authority, id, pior_nav, destino_usdc),
            "efetivar_resgate",
        );
    }

    /// Rotaciona a chave do oráculo.
    pub fn set_nav_oracle_raw(&mut self, signer: &Keypair, novo: Pubkey) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::SetNavOracle { novo }.data(),
            dom_vault::accounts::SetNavOracle {
                authority: signer.pubkey(),
                vault: vault_pda(),
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    /// **Atalho de harness**, não instrução: reduz o caixa direto na conta para
    /// simular o USDC que está com a mesa. Em F1 não existe instrução que tire
    /// dinheiro do cofre — a perna de trading é F2. Sai quando ela existir.
    pub fn forcar_caixa(&mut self, saldo_alvo: u64) {
        let mut conta = self.svm.get_account(&treasury_pda()).unwrap();
        // `amount` fica no offset 64 da conta de token SPL: mint(32) + owner(32).
        conta.data[64..72].copy_from_slice(&saldo_alvo.to_le_bytes());
        self.svm.set_account(treasury_pda(), conta).unwrap();
    }

    /// Lê a conta do cofre **crua**, sem desserializar. É o que permite afirmar
    /// o tamanho enquanto ele ainda é ilegível para o struct novo.
    pub fn bytes_do_cofre(&self) -> Vec<u8> {
        self.svm.get_account(&vault_pda()).unwrap().data
    }

    // -----------------------------------------------------------------------
    // Minimo de aporte no estado
    // -----------------------------------------------------------------------

    /// Ajusta um parametro de politica (Upgrade E). **Privilegiada.**
    pub fn ajustar_parametro_raw(
        &mut self,
        signer: &Keypair,
        qual: dom_vault::instructions::parametros::Parametro,
        novo: u64,
    ) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::AjustarParametro { qual, novo }.data(),
            dom_vault::accounts::AjustarParametro {
                authority: signer.pubkey(),
                vault: vault_pda(),
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    pub fn ajustar_parametro(
        &mut self,
        qual: dom_vault::instructions::parametros::Parametro,
        novo: u64,
    ) -> TransactionResult {
        let autoridade = self.authority.insecure_clone();
        self.ajustar_parametro_raw(&autoridade, qual, novo)
    }

    pub fn update_min_deposit_raw(&mut self, signer: &Keypair, novo: u64) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::UpdateMinDeposit { novo }.data(),
            dom_vault::accounts::UpdateMinDeposit {
                authority: signer.pubkey(),
                vault: vault_pda(),
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    pub fn update_min_deposit(&mut self, novo: u64) -> TransactionMetadata {
        let authority = self.authority.insecure_clone();
        assert_ok(
            self.update_min_deposit_raw(&authority, novo),
            "update_min_deposit",
        )
    }

    // -----------------------------------------------------------------------
    // Bloco 3 da F2: a porta operacional
    // -----------------------------------------------------------------------

    pub fn update_deploy_allowlist_raw(
        &mut self,
        signer: &Keypair,
        indice: u8,
        destino: Pubkey,
    ) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::UpdateDeployAllowlist { indice, destino }.data(),
            dom_vault::accounts::UpdateDeployAllowlist {
                authority: signer.pubkey(),
                vault: vault_pda(),
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    pub fn update_deploy_allowlist(&mut self, indice: u8, destino: Pubkey) -> TransactionMetadata {
        let authority = self.authority.insecure_clone();
        assert_ok(
            self.update_deploy_allowlist_raw(&authority, indice, destino),
            "update_deploy_allowlist",
        )
    }

    pub fn deploy_capital_raw(
        &mut self,
        signer: &Keypair,
        destino_usdc: Pubkey,
        valor: u64,
    ) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::DeployCapital { valor }.data(),
            dom_vault::accounts::DeployCapital {
                authority: signer.pubkey(),
                vault: vault_pda(),
                dom_mint: self.dom_mint,
                usdc_mint: self.usdc_mint,
                treasury: treasury_pda(),
                destino_usdc,
                usdc_token_program: token_classic(),
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    pub fn deploy_capital(&mut self, destino_usdc: Pubkey, valor: u64) -> TransactionMetadata {
        let authority = self.authority.insecure_clone();
        assert_ok(
            self.deploy_capital_raw(&authority, destino_usdc, valor),
            "deploy_capital",
        )
    }

    pub fn return_capital_raw(&mut self, origem: &Holder, valor: u64) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::ReturnCapital { valor }.data(),
            dom_vault::accounts::ReturnCapital {
                origem_authority: origem.wallet.pubkey(),
                vault: vault_pda(),
                usdc_mint: self.usdc_mint,
                origem_usdc: origem.usdc,
                treasury: treasury_pda(),
                usdc_token_program: token_classic(),
            }
            .to_account_metas(None),
        );
        let signer = origem.wallet.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    pub fn return_capital(&mut self, origem: &Holder, valor: u64) -> TransactionMetadata {
        assert_ok(self.return_capital_raw(origem, valor), "return_capital")
    }

    // -----------------------------------------------------------------------
    // Bloco 5: apuração, fee_share e pause
    // -----------------------------------------------------------------------

    /// Conta de USDC da autoridade — a origem do `P` no `deposit_especial`.
    ///
    /// Em produção é a ATA do Vault PDA do Squads: a mesa devolve o lucro da
    /// carteira operacional para lá com uma transferência comum, e a proposta
    /// executa a distribuição. Criada e abastecida sob demanda.
    pub fn caixa_da_autoridade(&mut self, usdc: u64) -> Pubkey {
        let dono = self.authority.pubkey();
        let conta = create_conta_usdc(&mut self.svm, &self.payer, &self.usdc_mint, &dono);
        if usdc > 0 {
            let usdc_mint = self.usdc_mint;
            let usdc_authority = self.usdc_authority.insecure_clone();
            mint_tokens(
                &mut self.svm,
                &self.payer,
                &token_classic(),
                &usdc_mint,
                &usdc_authority,
                &conta,
                usdc,
            );
        }
        conta
    }

    pub fn deposit_especial_raw(
        &mut self,
        signer: &Keypair,
        origem_usdc: Pubkey,
        lucro: u64,
    ) -> TransactionResult {
        let socios: Vec<Pubkey> = self.socios.iter().map(|s| s.wallet.pubkey()).collect();
        let contas: Vec<Pubkey> = self.socios.iter().map(|s| s.dom).collect();
        self.deposit_especial_com_destinos(signer, origem_usdc, lucro, &contas, &socios)
    }

    /// Versão explícita, para o teste que manda destinos duplicados (T61).
    pub fn deposit_especial_com_destinos(
        &mut self,
        signer: &Keypair,
        origem_usdc: Pubkey,
        lucro: u64,
        contas_dom: &[Pubkey],
        socios: &[Pubkey],
    ) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::DepositEspecial {
                lucro_realizado: lucro,
            }
            .data(),
            dom_vault::accounts::DepositEspecial {
                payer: self.payer.pubkey(),
                authority: signer.pubkey(),
                vault: vault_pda(),
                dom_mint: self.dom_mint,
                usdc_mint: self.usdc_mint,
                origem_usdc,
                treasury: treasury_pda(),
                socio0_dom: contas_dom[0],
                socio1_dom: contas_dom[1],
                socio2_dom: contas_dom[2],
                socio0_ledger: fee_share_pda(&socios[0]),
                socio1_ledger: fee_share_pda(&socios[1]),
                socio2_ledger: fee_share_pda(&socios[2]),
                dom_token_program: token_2022(),
                usdc_token_program: token_classic(),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    /// Distribui `lucro` de lucro realizado, abastecendo o caixa da autoridade
    /// com exatamente esse valor. É o caminho normal do fim de ciclo.
    pub fn deposit_especial(&mut self, lucro: u64) -> TransactionMetadata {
        let origem = self.caixa_da_autoridade(lucro);
        let authority = self.authority.insecure_clone();
        assert_ok(
            self.deposit_especial_raw(&authority, origem, lucro),
            "deposit_especial",
        )
    }

    pub fn sacar_lucro_raw(&mut self, cotista: &Holder) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::SacarLucro {}.data(),
            dom_vault::accounts::SacarLucro {
                cotista: cotista.wallet.pubkey(),
                vault: vault_pda(),
                marca: lucro_pda(&cotista.wallet.pubkey()),
                dom_mint: self.dom_mint,
                usdc_mint: self.usdc_mint,
                cotista_dom: cotista.dom,
                treasury: treasury_pda(),
                cotista_usdc: cotista.usdc,
                dom_token_program: token_2022(),
                usdc_token_program: token_classic(),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
        );
        let signer = cotista.wallet.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    pub fn sacar_lucro(&mut self, cotista: &Holder) -> TransactionMetadata {
        assert_ok(self.sacar_lucro_raw(cotista), "sacar_lucro")
    }

    pub fn redeem_fee_share_raw(&mut self, socio: &Holder, shares: u64) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::RedeemFeeShare { shares }.data(),
            dom_vault::accounts::RedeemFeeShare {
                socio: socio.wallet.pubkey(),
                vault: vault_pda(),
                ledger: fee_share_pda(&socio.wallet.pubkey()),
                dom_mint: self.dom_mint,
                usdc_mint: self.usdc_mint,
                socio_dom: socio.dom,
                treasury: treasury_pda(),
                socio_usdc: socio.usdc,
                dom_token_program: token_2022(),
                usdc_token_program: token_classic(),
            }
            .to_account_metas(None),
        );
        let wallet = socio.wallet.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&wallet])
    }

    pub fn redeem_fee_share(&mut self, socio: &Holder, shares: u64) -> TransactionMetadata {
        assert_ok(self.redeem_fee_share_raw(socio, shares), "redeem_fee_share")
    }

    fn set_pause_raw(&mut self, signer: &Keypair, pausar: bool) -> TransactionResult {
        let data = if pausar {
            dom_vault::instruction::Pause {}.data()
        } else {
            dom_vault::instruction::Unpause {}.data()
        };
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &data,
            dom_vault::accounts::SetPause {
                authority: signer.pubkey(),
                vault: vault_pda(),
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    pub fn pause_raw(&mut self, signer: &Keypair) -> TransactionResult {
        self.set_pause_raw(signer, true)
    }

    pub fn unpause_raw(&mut self, signer: &Keypair) -> TransactionResult {
        self.set_pause_raw(signer, false)
    }

    pub fn pause(&mut self) -> TransactionMetadata {
        let authority = self.authority.insecure_clone();
        assert_ok(self.pause_raw(&authority), "pause")
    }

    pub fn unpause(&mut self) -> TransactionMetadata {
        let authority = self.authority.insecure_clone();
        assert_ok(self.unpause_raw(&authority), "unpause")
    }

    pub fn update_socios_raw(
        &mut self,
        signer: &Keypair,
        socios: [Pubkey; 3],
    ) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            dom_vault::ID,
            &dom_vault::instruction::UpdateSocios { socios }.data(),
            dom_vault::accounts::UpdateSocios {
                authority: signer.pubkey(),
                vault: vault_pda(),
            }
            .to_account_metas(None),
        );
        let signer = signer.insecure_clone();
        send(&mut self.svm, &self.payer, &[ix], &[&signer])
    }

    pub fn saldo_escrow(&self) -> u64 {
        saldo(&self.svm, &escrow_dom_pda())
    }

    pub fn caixa(&self) -> u64 {
        saldo(&self.svm, &treasury_pda())
    }

    pub fn saldo_dom(&self, holder: &Holder) -> u64 {
        saldo(&self.svm, &holder.dom)
    }

    pub fn saldo_usdc(&self, holder: &Holder) -> u64 {
        saldo(&self.svm, &holder.usdc)
    }

    /// Saldo de uma conta de USDC avulsa — usada pelo T82b, que prova que a
    /// allowlist casa pelo **dono** e não pela conta.
    pub fn saldo_usdc_da_conta(&self, conta: Pubkey) -> u64 {
        saldo(&self.svm, &conta)
    }

    /// Segunda conta de USDC para uma carteira que já tem uma.
    pub fn conta_usdc_extra(&mut self, dono: &Pubkey) -> Pubkey {
        create_conta_usdc(&mut self.svm, &self.payer, &self.usdc_mint, dono)
    }

    pub fn supply_dom(&self) -> u64 {
        supply(&self.svm, &self.dom_mint)
    }
}
