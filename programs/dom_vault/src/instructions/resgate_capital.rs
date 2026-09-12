//! **Resgate de capital — D+180 como teto, pago pelo pior NAV do período.**
//!
//! Sucede a fila D+30 (`request_redeem` + `process_redemptions`), que saiu por
//! decisão da mesa em 2026-08-25: **um produto, não dois**. Enquanto os dois
//! existissem, ninguém usaria este — o investidor escolheria trinta dias contra
//! cento e oitenta, sempre.
//!
//! # O contrato é burro, e é de propósito
//!
//! Ele **não** lê health factor, **não** toma empréstimo, **não** desmonta
//! posição. De onde sai o dinheiro — empréstimo novo, saque de empréstimo, lucro
//! de trading — é decisão da mesa, fora do cofre, e ela abastece o
//! `endereco_resgate`. O contrato confere se aquela conta tem saldo e paga.
//!
//! É essa burrice que permite o cofre ser pequeno o bastante para ser auditado.
//!
//! # As quatro instruções
//!
//! | | quem chama | o que faz |
//! |---|---|---|
//! | `solicitar_resgate_capital` | o cotista | trava as cotas no escrow e abre o pedido |
//! | `carimbar_nav` | **qualquer um** | desce a catraca do pior-NAV |
//! | `marcar_vencido` | **qualquer um** | passado o prazo, trava o `deploy_capital` |
//! | `efetivar_resgate_capital` | a mesa, por proposta | queima a cota e paga |
use {
    crate::{
        constants::*,
        error::DomError,
        events::{ResgateEfetivado, ResgateEmAtraso, ResgateSolicitado},
        math::{require_nav_fresco, usdc_from_shares},
        state::{PedidoResgate, Vault},
        whitelist::require_whitelisted,
    },
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{
        burn, transfer_checked, Burn, Mint, TokenAccount, TokenInterface, TransferChecked,
    },
    solana_instructions_sysvar::get_instruction_relative,
};

/// Primeiro byte do `TransferChecked` do Token-2022.
const TRANSFER_CHECKED_TAG: u8 = 12;

/// Estados do pedido. Não é enum Borsh de propósito: `u8` cru sobrevive a
/// variante nova sem mudar o tamanho da conta.
pub const PEDIDO_ABERTO: u8 = 0;
pub const PEDIDO_PAGO: u8 = 1;
pub const PEDIDO_EM_ATRASO: u8 = 2;

// ===========================================================================
// 1. Solicitar — o cotista assina
// ===========================================================================

#[derive(Accounts)]
pub struct SolicitarResgateCapital<'info> {
    #[account(mut)]
    pub cotista: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = dom_mint @ DomError::UnknownMint,
        has_one = escrow_dom @ DomError::InvalidEscrowAccount,
        constraint = !vault.paused @ DomError::Paused,
    )]
    pub vault: Box<Account<'info, Vault>>,

    /// Fail-closed do hook: quem não está na whitelist não pede.
    ///
    /// `UncheckedAccount` como no `deposit`: quem confere é o
    /// `require_whitelisted`, que deriva a PDA, confere o dono e desserializa.
    /// Declarar como `Account<WhitelistEntry>` faria a conta AUSENTE virar erro
    /// de conta, não `NotWhitelisted` — e a função é uma só de propósito.
    /// CHECK: conferida por `require_whitelisted`.
    pub cotista_whitelist: UncheckedAccount<'info>,

    /// A conta do pedido. Semeada pelo contador global do cofre.
    #[account(
        init,
        payer = cotista,
        space = 8 + PedidoResgate::INIT_SPACE,
        seeds = [RESGATE_SEED, &vault.proximo_pedido_id.to_le_bytes()],
        bump,
    )]
    pub pedido: Box<Account<'info, PedidoResgate>>,

    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        mut,
        token::mint = dom_mint,
        token::authority = cotista,
        token::token_program = dom_token_program,
    )]
    pub cotista_dom: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub escrow_dom: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
    /// CHECK: sysvar de instruções, conferido pelo endereço. É o que permite ler
    /// a instrução anterior e provar que a transferência para o escrow é desta
    /// transação.
    #[account(address = solana_instructions_sysvar::ID)]
    pub instructions_sysvar: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

/// **Não chama `require_nav_fresco`, e isso é a D-F2-03 sendo respeitada.**
///
/// > **Pedido de resgate é direito do cotista — não fica refém do backend.**
///
/// O que a D-F2-03 protege é o **direito de pedir**, e ele segue protegido: NAV
/// vencido **não impede** abrir pedido. O `nav_na_solicitacao` é gravado como
/// está — é **teto** do pior-NAV, não preço. Preço é o que a mesa informa na
/// efetivação, com `require_nav_fresco` e três travas por cima.
///
/// **O Upgrade D passou o piso para USDC, e não fere isso.** A instrução já lia
/// `vault.nav` para gravar o `nav_na_solicitacao`; comparar contra o mesmo número
/// não acrescenta dependência do oráculo. E como aqui **nenhum dinheiro se move**,
/// NAV velho no portão de admissão não paga errado: no máximo admite ou barra um
/// pedido na borda. O que o piso em COTAS fazia — travar para sempre quem comprou
/// menos de 1.000 — era pior, e não era reversível.
pub fn handle_solicitar_resgate_capital(
    ctx: Context<SolicitarResgateCapital>,
    cotas: u64,
) -> Result<()> {
    let cotista = ctx.accounts.cotista.key();
    require_whitelisted(&ctx.accounts.cotista_whitelist, &cotista)?;

    // Piso do Upgrade D: DOIS, ligados por OU. Um passa, entra.
    //
    // Em USDC mede `cotas × nav` — CAPITAL + valorizacao, a operacao inteira,
    // diferente do piso do saque de lucro, que mede so' o lucro do ciclo. E' a
    // porta de quem comprou pouco e viu a cota valorizar.
    //
    // Em COTAS e' a trava ANTI-COLAPSO, e ela e' imune ao preco. Sozinho, o piso
    // em USDC fechava o resgate justamente quando o NAV caia: com queda de 20%,
    // 1.000 cotas valiam 800 e o pedido era recusado. Os tripwires T75 e T90
    // pegaram isso — e e' por eles que este OU existe.
    let valor_pedido = usdc_from_shares(cotas, ctx.accounts.vault.nav)?;
    require!(
        cotas >= ctx.accounts.vault.min_resgate_capital_cotas
            || valor_pedido >= ctx.accounts.vault.min_resgate_capital_usdc,
        DomError::ResgateAbaixoDoMinimo
    );

    // -----------------------------------------------------------------------
    // **NÃO há trava de janela de lucro aqui, e é decisão, não esquecimento.**
    //
    // O `deposit` e a transferência entre carteiras recusam com a janela aberta
    // (D-F2-09), porque nos dois casos alguém ganharia direito que não gerou.
    // Aqui é o contrário: a cota vai para o escrow do programa, que é destino
    // isento no hook justamente porque cota entrando ali **reduz** o direito de
    // quem pediu e não cria direito para ninguém.
    //
    // Fechar o pedido durante a janela seria confundir fechar a janela com
    // sequestrar o direito do cotista — a distinção é da D-F2-03, e o resgate de
    // capital herda esse princípio inteiro do `request_redeem` que ele sucede.
    //
    // Custo para quem pede: cota em escrow não conta no `sacar_lucro`, que lê o
    // saldo da carteira. Quem resgata durante a janela abre mão do saque daquela
    // distribuição sobre a fração travada. É escolha dele, e é honesta.
    // -----------------------------------------------------------------------

    let now = Clock::get()?.unix_timestamp;
    let id = ctx.accounts.vault.proximo_pedido_id;
    let nav = ctx.accounts.vault.nav;

    // -----------------------------------------------------------------------
    // O resgate é COMPOSTO (D17): a transferência das cotas para o escrow é a
    // instrução IMEDIATAMENTE ANTERIOR, na mesma transação, e aqui se prova isso
    // por introspecção.
    //
    // **Por que não um CPI daqui.** O DOM tem transfer hook. Um
    // `transfer_checked` por CPI dispararia o `execute`, que precisa das contas
    // extras do `ExtraAccountMetaList` — e CPI não as resolve sozinho. Ou o
    // programa passa a carregar a plumbing do hook para dentro de si, ou o
    // cliente monta a transferência e o programa confere. A segunda é o padrão
    // que a F1 já provou e auditou, e é a que fica.
    //
    // Amarrar ao índice -1 é o que impede reaproveitar uma transferência para
    // dois pedidos: o -1 do segundo seria o primeiro pedido, não a transferência.
    // -----------------------------------------------------------------------
    let anterior = get_instruction_relative(-1, &ctx.accounts.instructions_sysvar)
        .map_err(|_| error!(DomError::EscrowTransferMissing))?;

    require_keys_eq!(
        anterior.program_id,
        anchor_spl::token_2022::ID,
        DomError::EscrowTransferMissing
    );
    require!(
        anterior.data.len() >= 10 && anterior.data[0] == TRANSFER_CHECKED_TAG,
        DomError::EscrowTransferMissing
    );
    let movido = u64::from_le_bytes(
        anterior.data[1..9]
            .try_into()
            .map_err(|_| error!(DomError::EscrowTransferMissing))?,
    );
    require!(movido == cotas, DomError::EscrowTransferMissing);

    // Ordem das contas do `TransferChecked`: origem, mint, destino, autoridade.
    require!(
        anterior.accounts.len() >= 4,
        DomError::EscrowTransferMissing
    );
    require_keys_eq!(
        anterior.accounts[0].pubkey,
        ctx.accounts.cotista_dom.key(),
        DomError::EscrowTransferMissing
    );
    require_keys_eq!(
        anterior.accounts[1].pubkey,
        ctx.accounts.dom_mint.key(),
        DomError::EscrowTransferMissing
    );
    require_keys_eq!(
        anterior.accounts[2].pubkey,
        ctx.accounts.escrow_dom.key(),
        DomError::EscrowTransferMissing
    );
    require_keys_eq!(
        anterior.accounts[3].pubkey,
        cotista,
        DomError::EscrowTransferMissing
    );

    // Cinto e suspensório: a introspecção prova que a transferência desta
    // transação existe; o saldo prova que o escrow cobre tudo que já está
    // travado. Se um dia a primeira falhar por um encoding novo, a segunda ainda
    // barra pedido sem lastro.
    let travadas = ctx
        .accounts
        .vault
        .cotas_travadas_resgate
        .checked_add(cotas)
        .ok_or(DomError::MathOverflow)?;
    require!(
        ctx.accounts.escrow_dom.amount >= travadas,
        DomError::EscrowShortfall
    );

    let pedido = &mut ctx.accounts.pedido;
    pedido.owner = cotista;
    pedido.id = id;
    pedido.cotas = cotas;
    pedido.solicitado_ts = now;
    pedido.vence_ts = now
        .checked_add(ctx.accounts.vault.resgate_capital_prazo)
        .ok_or(DomError::MathOverflow)?;
    pedido.nav_na_solicitacao = nav;
    pedido.pior_nav_visto = nav;
    pedido.estado = PEDIDO_ABERTO;
    pedido.bump = ctx.bumps.pedido;

    let vault = &mut ctx.accounts.vault;
    vault.proximo_pedido_id = id.checked_add(1).ok_or(DomError::MathOverflow)?;
    vault.cotas_travadas_resgate = vault
        .cotas_travadas_resgate
        .checked_add(cotas)
        .ok_or(DomError::MathOverflow)?;

    emit!(ResgateSolicitado {
        id,
        owner: cotista,
        cotas,
        nav_na_solicitacao: nav,
        solicitado_ts: now,
        vence_ts: pedido.vence_ts,
    });
    Ok(())
}

// ===========================================================================
// 2. Carimbar NAV — a catraca, permissionless
// ===========================================================================

#[derive(Accounts)]
pub struct CarimbarNav<'info> {
    #[account(seeds = [VAULT_SEED], bump = vault.bump)]
    pub vault: Box<Account<'info, Vault>>,
    #[account(
        mut,
        seeds = [RESGATE_SEED, &pedido.id.to_le_bytes()],
        bump = pedido.bump,
    )]
    pub pedido: Box<Account<'info, PedidoResgate>>,
}

/// **Sem signatário de propósito.** O backend carimba a cada publicação de NAV;
/// o investidor carimba o próprio pedido se desconfiar. Quem carimba não escolhe
/// o número — ele vem do `vault.nav` do instante — e a catraca **só desce**.
///
/// Chamar com NAV alto não faz nada. É o que torna seguro deixar aberto.
///
/// # **Esta instrução NÃO chama `require_nav_fresco`, e é de propósito**
///
/// Leia isto antes de "consertar": em 2026-08-26 uma ausência parecida foi achada
/// no `redeem_fee_share` e **era erro** — pagava a taxa do sócio ao NAV vencido.
/// Foi corrigida no Upgrade C. Quem varrer o código atrás de outras vai parar aqui
/// e achar que encontrou a segunda. **Não encontrou.** As duas ausências parecem
/// iguais por fora e são opostas no mérito:
///
/// | | `redeem_fee_share` (D4) | `carimbar_nav` |
/// |---|---|---|
/// | move dinheiro? | **sim**, paga USDC ao sócio | **não**, só desce um contador |
/// | NAV velho causa o quê? | **pagamento a mais**, do treasury | registra um NAV que **de fato ocorreu** |
/// | travar seria | **correto** — e foi feito | **perverso** — ver abaixo |
///
/// **Por que travar aqui seria perverso.** A catraca existe para capturar o
/// **menor NAV do período**, e as leituras baixas aparecem justamente quando o
/// oráculo está caindo ou parado — que é exatamente quando o NAV fica "vencido".
/// Exigir frescura aqui **cegaria a catraca no momento para o qual ela serve**.
///
/// NAV vencido não é número inventado: é o último preço publicado, e ele
/// **ocorreu**. Registrá-lo é o trabalho desta instrução.
///
/// E quem **paga** é o `efetivar_resgate_capital` — esse exige NAV fresco, e é lá
/// que a trava tem de estar.
pub fn handle_carimbar_nav(ctx: Context<CarimbarNav>) -> Result<()> {
    let nav = ctx.accounts.vault.nav;
    let pedido = &mut ctx.accounts.pedido;
    require!(pedido.estado != PEDIDO_PAGO, DomError::ResgateJaPago);
    if nav < pedido.pior_nav_visto {
        pedido.pior_nav_visto = nav;
    }
    Ok(())
}

// ===========================================================================
// 3. Marcar vencido — o dente da obrigação firme, permissionless
// ===========================================================================

#[derive(Accounts)]
pub struct MarcarVencido<'info> {
    #[account(mut, seeds = [VAULT_SEED], bump = vault.bump)]
    pub vault: Box<Account<'info, Vault>>,
    #[account(
        mut,
        seeds = [RESGATE_SEED, &pedido.id.to_le_bytes()],
        bump = pedido.bump,
    )]
    pub pedido: Box<Account<'info, PedidoResgate>>,
}

/// Passado o prazo sem pagamento, marca o pedido e **trava o `deploy_capital`**.
///
/// O contrato não consegue obrigar a mesa a desmontar posição para honrar o
/// prazo. Consegue tornar o descumprimento caro: enquanto houver pedido vencido
/// não pago, o cofre não manda mais capital para campo. Não se aumenta a aposta
/// devendo a quem pediu para sair.
///
/// **Permissionless de propósito** — quem aciona é o próprio prejudicado, sem
/// depender da boa vontade de quem está em falta.
pub fn handle_marcar_vencido(ctx: Context<MarcarVencido>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let pedido = &mut ctx.accounts.pedido;

    require!(pedido.estado != PEDIDO_PAGO, DomError::ResgateJaPago);
    require!(
        pedido.estado != PEDIDO_EM_ATRASO,
        DomError::ResgateJaEmAtraso
    );
    require!(now > pedido.vence_ts, DomError::ResgateNaoVenceu);

    pedido.estado = PEDIDO_EM_ATRASO;
    let vault = &mut ctx.accounts.vault;
    vault.resgates_em_atraso = vault
        .resgates_em_atraso
        .checked_add(1)
        .ok_or(DomError::MathOverflow)?;

    emit!(ResgateEmAtraso {
        id: pedido.id,
        owner: pedido.owner,
        cotas: pedido.cotas,
        vence_ts: pedido.vence_ts,
        marcado_ts: now,
        total_em_atraso: vault.resgates_em_atraso,
    });
    Ok(())
}

// ===========================================================================
// 4. Efetivar — a mesa, por proposta
// ===========================================================================

#[derive(Accounts)]
pub struct EfetivarResgateCapital<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
        has_one = dom_mint @ DomError::UnknownMint,
        has_one = usdc_mint @ DomError::UnknownUsdcMint,
        has_one = escrow_dom @ DomError::InvalidEscrowAccount,
        has_one = endereco_resgate @ DomError::EnderecoDeResgateInvalido,
    )]
    pub vault: Box<Account<'info, Vault>>,

    #[account(
        mut,
        seeds = [RESGATE_SEED, &pedido.id.to_le_bytes()],
        bump = pedido.bump,
    )]
    pub pedido: Box<Account<'info, PedidoResgate>>,

    #[account(mut)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(mut)]
    pub escrow_dom: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: PDA de autoridade do escrow; só assina o burn.
    #[account(seeds = [ESCROW_SEED], bump = vault.escrow_bump)]
    pub escrow_authority: UncheckedAccount<'info>,

    /// A conta de onde o resgate é pago. Fixada em estado, e a autoridade dela
    /// tem de ser a `authority` — o mesmo quórum que aprovou a proposta.
    #[account(
        mut,
        token::mint = usdc_mint,
        token::authority = authority,
        token::token_program = usdc_token_program,
    )]
    pub endereco_resgate: Box<InterfaceAccount<'info, TokenAccount>>,

    /// Só o dono do pedido recebe.
    #[account(
        mut,
        token::mint = usdc_mint,
        token::authority = pedido.owner,
        token::token_program = usdc_token_program,
    )]
    pub cotista_usdc: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(address = anchor_spl::token_2022::ID @ DomError::MintNotToken2022)]
    pub dom_token_program: Interface<'info, TokenInterface>,
    pub usdc_token_program: Interface<'info, TokenInterface>,
}

/// **Não exige que o prazo tenha vencido.** O D+180 é teto, não carência: a mesa
/// antecipa quando a saúde da posição permitir (decisão 1 e 2 da mesa), e o
/// critério de antecipação é dela, fora do contrato.
///
/// # Sobre o `pior_nav`
///
/// O contrato **não tem** o histórico de NAV e **não pode** verificar que o
/// número é o mínimo do período. Ele limita por cima, três vezes:
/// `nav_na_solicitacao`, `nav` corrente e a catraca `pior_nav_visto`.
///
/// Repare a direção do risco: pior-NAV **menor paga menos** ao investidor. O
/// limite superior protege quem **fica**, impedindo pagar demais e diluir os
/// outros. Ele **não protege quem sai** — um número artificialmente baixo passa.
///
/// Quem sai é protegido por **verificabilidade, não por trava**: o
/// `ResgateEfetivado` carrega `pior_nav`, `solicitado_ts` e `efetivado_ts`, e o
/// `NavPublished` carrega cada NAV com atestação. Enumerar as publicações do
/// período e recalcular o mínimo é mecânico — divergência é fraude demonstrável.
///
/// É por isso que o indexador durável de `NavPublished` é pré-requisito duro, e
/// não conveniência: evento expira da retenção do RPC, e com ele evapora, em
/// silêncio, a única proteção de quem sai.
pub fn handle_efetivar_resgate_capital(
    ctx: Context<EfetivarResgateCapital>,
    pior_nav: u64,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    // **NAV velho para o pagamento** — e aqui, ao contrário do pedido, a trava
    // faz sentido. O `pior_nav` é limitado por `vault.nav`, então NAV morto
    // afrouxa o limite e deixa pagar acima do que o fundo vale hoje: quem sai
    // leva demais e quem fica paga a conta. É a mesma decisão do T72 sobre a
    // fila antiga, no mecanismo novo.
    //
    // Não trava nada de forma definitiva: a válvula da mesa publica NAV sem
    // intervalo mínimo e sem gate de pausa, então a saída está sempre a dois
    // votos de distância — os mesmos dois que aprovaram esta efetivação.
    require_nav_fresco(
        ctx.accounts.vault.nav_ts,
        now,
        ctx.accounts.vault.max_nav_staleness,
    )?;

    require!(
        ctx.accounts.pedido.estado != PEDIDO_PAGO,
        DomError::ResgateJaPago
    );
    require!(pior_nav > 0, DomError::PiorNavZero);
    require!(
        pior_nav <= ctx.accounts.pedido.nav_na_solicitacao,
        DomError::PiorNavAcimaDoTeto
    );
    require!(
        pior_nav <= ctx.accounts.vault.nav,
        DomError::PiorNavAcimaDoTeto
    );
    require!(
        pior_nav <= ctx.accounts.pedido.pior_nav_visto,
        DomError::PiorNavAcimaDoTeto
    );

    let cotas = ctx.accounts.pedido.cotas;
    let valor = usdc_from_shares(cotas, pior_nav)?;
    require!(valor > 0, DomError::ZeroRedeemValue);
    require!(
        ctx.accounts.endereco_resgate.amount >= valor,
        DomError::SemSaldoNoEnderecoDeResgate
    );

    // A cota morre. Não volta para a carteira, não fica no escrow.
    let escrow_bump = ctx.accounts.vault.escrow_bump;
    let escrow_seeds: &[&[&[u8]]] = &[&[ESCROW_SEED, &[escrow_bump]]];
    burn(
        CpiContext::new_with_signer(
            ctx.accounts.dom_token_program.key(),
            Burn {
                mint: ctx.accounts.dom_mint.to_account_info(),
                from: ctx.accounts.escrow_dom.to_account_info(),
                authority: ctx.accounts.escrow_authority.to_account_info(),
            },
            escrow_seeds,
        ),
        cotas,
    )?;

    // O dinheiro vem de FORA do cofre: o treasury não é tocado.
    transfer_checked(
        CpiContext::new(
            ctx.accounts.usdc_token_program.key(),
            TransferChecked {
                from: ctx.accounts.endereco_resgate.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.cotista_usdc.to_account_info(),
                authority: ctx.accounts.authority.to_account_info(),
            },
        ),
        valor,
        ctx.accounts.usdc_mint.decimals,
    )?;

    let estava_em_atraso = ctx.accounts.pedido.estado == PEDIDO_EM_ATRASO;
    let pedido = &mut ctx.accounts.pedido;
    pedido.estado = PEDIDO_PAGO;
    let id = pedido.id;
    let owner = pedido.owner;
    let solicitado_ts = pedido.solicitado_ts;

    let vault = &mut ctx.accounts.vault;
    vault.cotas_travadas_resgate = vault
        .cotas_travadas_resgate
        .checked_sub(cotas)
        .ok_or(DomError::MathOverflow)?;
    if estava_em_atraso {
        vault.resgates_em_atraso = vault.resgates_em_atraso.saturating_sub(1);
    }

    emit!(ResgateEfetivado {
        id,
        owner,
        cotas,
        pior_nav,
        valor,
        solicitado_ts,
        efetivado_ts: now,
        nav_attestation: vault.nav_attestation,
    });
    Ok(())
}

// ===========================================================================
// 5. Nomear o endereço de resgate — a mesa
// ===========================================================================

#[derive(Accounts)]
pub struct UpdateEnderecoResgate<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
        has_one = usdc_mint @ DomError::UnknownUsdcMint,
    )]
    pub vault: Box<Account<'info, Vault>>,
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,
    /// A conta que vai pagar os resgates. **Autoridade tem de ser a `authority`**
    /// — o Vault do Squads —, igual à `origem_usdc` do `deposit_especial`.
    ///
    /// Fixar em estado, em vez de aceitar qualquer conta do Squads na hora de
    /// pagar, é o que permite ao dashboard mostrar "o fundo de resgate tem X"
    /// com significado, e o que impede a mesa pagar da conta errada por engano.
    #[account(
        token::mint = usdc_mint,
        token::authority = authority,
        token::token_program = usdc_token_program,
    )]
    pub novo_endereco: Box<InterfaceAccount<'info, TokenAccount>>,
    pub usdc_token_program: Interface<'info, TokenInterface>,
}

pub fn handle_update_endereco_resgate(ctx: Context<UpdateEnderecoResgate>) -> Result<()> {
    let anterior = ctx.accounts.vault.endereco_resgate;
    let novo = ctx.accounts.novo_endereco.key();
    ctx.accounts.vault.endereco_resgate = novo;
    emit!(crate::events::EnderecoDeResgateAtualizado { anterior, novo });
    Ok(())
}
