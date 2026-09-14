use {
    crate::{constants::*, error::DomError, events::VaultInitialized, state::Vault},
    anchor_lang::{prelude::*, solana_program::program_option::COption},
    anchor_spl::token_interface::{
        spl_token_2022::{
            extension::{
                transfer_hook::TransferHook, BaseStateWithExtensions, StateWithExtensions,
            },
            state::Mint as Token2022Mint,
        },
        Mint, TokenAccount, TokenInterface,
    },
};

/// Quase toda conta aqui é `Box`: o frame de stack do BPF tem 4 KB e esta
/// instrução carrega dois mints, duas contas de token, o cofre e a fila. Sem
/// `Box` o `try_accounts` estoura a pilha — `Access violation in stack frame`,
/// que foi o que aconteceu na primeira versão deste bloco.
#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// Vault PDA do Squads v4 em produção (D2). Em teste, uma chave qualquer —
    /// a instrução não distingue, e não deve: quem assina é a autoridade.
    pub authority: Signer<'info>,
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    /// USDC aceito em `deposit`. Fica gravado no cofre: allowlist de um item,
    /// como o mint do DOM.
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init,
        payer = payer,
        space = 8 + Vault::INIT_SPACE,
        seeds = [VAULT_SEED],
        bump
    )]
    pub vault: Box<Account<'info, Vault>>,
    /// Caixa do fundo. Nasce junto do cofre para que não exista janela entre
    /// "cofre inicializado" e "cofre com lugar para receber USDC".
    #[account(
        init,
        payer = payer,
        seeds = [TREASURY_SEED],
        bump,
        token::mint = usdc_mint,
        token::authority = vault,
        token::token_program = usdc_token_program,
    )]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: PDA que fica como autoridade das contas do programa. Não guarda
    /// dado — só assina.
    #[account(seeds = [ESCROW_SEED], bump)]
    pub escrow_authority: UncheckedAccount<'info>,
    /// Cotas em escrow entre pedido e processamento (D8).
    #[account(
        init,
        payer = payer,
        seeds = [ESCROW_DOM_SEED],
        bump,
        token::mint = dom_mint,
        token::authority = escrow_authority,
        token::token_program = dom_token_program,
    )]
    pub escrow_dom: Box<InterfaceAccount<'info, TokenAccount>>,
    pub dom_token_program: Interface<'info, TokenInterface>,
    pub usdc_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize(
    ctx: Context<Initialize>,
    nav_oracle: Pubkey,
    socios: [Pubkey; NUM_SOCIOS],
) -> Result<()> {
    // D10: sócios distintos par a par, aqui e em toda instrução que escreva o
    // campo. Defesa na configuração, não falha tardia na apuração (T57).
    crate::instructions::update_socios::require_socios_distintos(&socios)?;

    let authority = ctx.accounts.authority.key();
    let vault_key = ctx.accounts.vault.key();
    let mint_info = ctx.accounts.dom_mint.to_account_info();

    // O mint precisa ser Token-2022: o hook só existe lá. `InterfaceAccount`
    // aceita os dois programas de token, então a checagem é explícita.
    require_keys_eq!(
        *mint_info.owner,
        anchor_spl::token_2022::ID,
        DomError::MintNotToken2022
    );

    // -----------------------------------------------------------------------
    // Autoridade de mint no PDA do cofre (D15).
    //
    // Sem isso o `deposit` não consegue mintar cota nenhuma — e descobriria
    // isso no CPI, em runtime, com erro do Token-2022 em vez de erro do cofre.
    // A conferência aqui é o que garante que um cofre inicializado é um cofre
    // que funciona.
    // -----------------------------------------------------------------------
    require!(
        ctx.accounts.dom_mint.mint_authority == COption::Some(vault_key),
        DomError::MintAuthorityMismatch
    );
    // `None` (congelamento impossível) é mais forte que o exigido e passa.
    let freeze_ok = match ctx.accounts.dom_mint.freeze_authority {
        COption::None => true,
        COption::Some(key) => key == vault_key,
    };
    require!(freeze_ok, DomError::FreezeAuthorityMismatch);

    // Amarra o cofre a um mint que já aponta o hook para este programa. Sem
    // isso o cofre poderia nascer ligado a um mint sem hook — whitelist e cap
    // viram decoração e ninguém percebe até a primeira transferência (D2/D5).
    {
        let data = mint_info.try_borrow_data()?;
        let state = StateWithExtensions::<Token2022Mint>::unpack(&data)
            .map_err(|_| error!(DomError::TransferHookNotConfigured))?;
        let hook = state
            .get_extension::<TransferHook>()
            .map_err(|_| error!(DomError::TransferHookNotConfigured))?;

        let hook_program: Option<Pubkey> = hook.program_id.into();
        require!(
            hook_program == Some(crate::ID),
            DomError::TransferHookNotConfigured
        );

        // `None` = hook imutável, que é mais forte que o exigido. Qualquer
        // outra chave significa que um terceiro pode trocar o hook — T23.
        let hook_authority: Option<Pubkey> = hook.authority.into();
        require!(
            hook_authority.is_none_or(|a| a == authority),
            DomError::TransferHookAuthorityMismatch
        );
    }

    let treasury = ctx.accounts.treasury.key();
    let escrow_authority = ctx.accounts.escrow_authority.key();
    let escrow_dom = ctx.accounts.escrow_dom.key();
    let usdc_mint = ctx.accounts.usdc_mint.key();
    let dom_mint = ctx.accounts.dom_mint.key();
    let treasury_bump = ctx.bumps.treasury;
    let escrow_bump = ctx.bumps.escrow_authority;
    let bump = ctx.bumps.vault;
    let vault = &mut ctx.accounts.vault;
    vault.authority = authority;
    vault.nav_oracle = nav_oracle;
    vault.socios = socios;
    vault.dom_mint = dom_mint;
    vault.usdc_mint = usdc_mint;
    vault.treasury = treasury;
    vault.escrow_dom = escrow_dom;
    vault.escrow_authority = escrow_authority;
    vault.nav = NAV_GENESIS; // 1,000000 — primeiro depósito sai 1:1 (T01)
    vault.nav_ts = 0;
    vault.cotas_travadas_resgate = 0;
    vault.reserve_bps = RESERVE_BPS; // 10% (D7)
    vault.paused = false;
    vault.cap_enforced = false; // gênese (D3)
                                // --- bloco de estado da F2 ---
    vault.nav_attestation = [0u8; 32]; // gênese não tem laudo: o NAV é o de partida
    vault.ultima_distribuicao_ts = 0; // nunca distribuiu — a primeira não espera
    vault.deployed_usdc = 0;
    // --- bloco do resgate de capital ---
    // Nasce em zero: o primeiro pedido leva o id 0. Endereço de resgate nasce
    // VAZIO, como a allowlist — a mesa o nomeia por proposta, e sem ele nenhum
    // resgate se efetiva. Capital novo nasce preso dos dois lados.
    vault.proximo_pedido_id = 0;
    vault.resgates_em_atraso = 0;
    vault.endereco_resgate = Pubkey::default();
    // -----------------------------------------------------------------------
    // PARÂMETROS DE POLÍTICA — Upgrade E (D-F2-21). Nascem da constante.
    // -----------------------------------------------------------------------
    // A constante deixou de mandar no cofre vivo e passou a ser **o valor de
    // partida**: aqui, e na migração que já rodou nos cofres vivos. Daqui em
    // diante quem manda é o campo, mudado por `ajustar_parametro` — proposta,
    // não upgrade.
    //
    // Esquecer uma linha destas não daria erro de compilação: o campo ficaria
    // em zero, e zero em `max_nav_staleness` fecha o cofre inteiro no primeiro
    // `deposit`. É o que `tests/estado_f2.rs` confere campo a campo depois da
    // gênese, e não por gosto de teste redundante.
    vault.resgate_capital_prazo = RESGATE_CAPITAL_PRAZO;
    vault.min_resgate_capital_usdc = MIN_RESGATE_CAPITAL_USDC;
    vault.min_resgate_capital_cotas = MIN_RESGATE_CAPITAL_COTAS;
    vault.min_saque_lucro_usdc = MIN_SAQUE_LUCRO_USDC;
    vault.max_nav_staleness = MAX_NAV_STALENESS;
    vault.min_nav_publish_interval = MIN_NAV_PUBLISH_INTERVAL;
    vault.distribuicao_interval = DISTRIBUICAO_INTERVAL;
    vault.nav_bound_pct = NAV_BOUND_NUM as u16;
    vault.cap_pct = CAP_NUM as u16;
    vault.perf_fee_bps_total = PERF_FEE_BPS_TOTAL;
    vault.layout_version = LAYOUT_VERSION;
    // Allowlist **vazia** na gênese: `deploy_capital` não tem para onde mandar
    // antes de a mesa registrar destino por proposta. Capital novo nasce preso.
    vault.deploy_allowlist = [Pubkey::default(); DEPLOY_ALLOWLIST_LEN];
    // Piso de aporte de gênese. Daqui em diante quem manda é o campo, mudado
    // por proposta — nao por upgrade. **Mainnet nasce com ele**, e por isso a
    // instrução de migração nao precisa existir la'.
    vault.min_deposit = MIN_DEPOSIT;
    // --- bloco do rendimento (D-F2-09) ---
    // Gênese não tem distribuição atrás dela: nada rendeu por cota, nada há a
    // sacar, janela fechada. **Mainnet nasce assim** — é por isto que a
    // `migrate_vault_lucro` só serve ao cofre de devnet e sai antes da firma.
    vault.delta_lucro_por_cota = 0;
    vault.lucro_sacavel_restante = 0;
    vault.bump = bump;
    vault.escrow_bump = escrow_bump;
    vault.treasury_bump = treasury_bump;

    emit!(VaultInitialized {
        authority,
        nav_oracle,
        socios,
        dom_mint,
        usdc_mint,
        treasury,
        escrow_authority,
        nav: NAV_GENESIS,
    });

    Ok(())
}
