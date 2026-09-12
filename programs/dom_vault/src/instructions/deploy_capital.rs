use {
    crate::{
        constants::*,
        error::DomError,
        events::{CapitalDeployed, CapitalReturned, JanelaDeLucroEncerrada},
        math::{require_nav_fresco, usdc_from_shares},
        state::Vault,
    },
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{
        transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
    },
};

/// Move USDC da treasury para um destino operacional.
///
/// **É a porta que a F1 não tinha.** A F1 provou um cofre fechado: dinheiro
/// entrava por `deposit` e só saía por resgate ou taxa, para destinos
/// **derivados** — o dono do pedido na fila, o sócio do livro. Esta é a primeira
/// saída para endereço de **lista**, com valor livre, e por isso carrega três
/// travas em vez de uma.
///
/// Consequência para a auditoria externa: o modelo de ameaças da revisão interna
/// descreve o cofre fechado. Esta instrução muda a peça central dele.
#[derive(Accounts)]
pub struct DeployCapital<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
        has_one = usdc_mint @ DomError::UnknownUsdcMint,
        has_one = treasury @ DomError::InvalidTreasury,
        has_one = dom_mint @ DomError::UnknownMint,
        // Cofre pausado não deixa capital sair. `pause` é o freio de crise, e
        // mandar dinheiro para o trabalho no meio de uma é o oposto do que ele
        // existe para fazer. A volta (`return_capital`) continua liberada: trazer
        // dinheiro de casa nunca precisou de permissão.
        constraint = !vault.paused @ DomError::Paused,
    )]
    pub vault: Box<Account<'info, Vault>>,
    /// Só para ler o `supply` — a reserva é percentual do patrimônio.
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Conta de USDC do destino. O **owner** dela é que precisa estar na
    /// allowlist — nunca a conta em si (D-F2-02).
    #[account(mut)]
    pub destino_usdc: Box<InterfaceAccount<'info, TokenAccount>>,
    pub usdc_token_program: Interface<'info, TokenInterface>,
}

pub fn handle_deploy_capital(ctx: Context<DeployCapital>, valor: u64) -> Result<()> {
    require!(valor > 0, DomError::ZeroDeploy);

    // -----------------------------------------------------------------------
    // TRAVA 0 — resgate vencido não pago para o capital de sair.
    // -----------------------------------------------------------------------
    // O prazo de 180 dias é obrigação firme: se as outras fontes não cobrirem, a
    // mesa desmonta posição para honrar. O contrato não consegue obrigar ninguém
    // a desmontar nada — consegue tornar o descumprimento caro.
    //
    // **Não se aumenta a aposta devendo a quem pediu para sair.** Enquanto
    // houver pedido vencido não pago, nada de capital novo em campo.
    //
    // O contador sobe por `marcar_vencido`, que é permissionless: quem aciona é
    // o próprio prejudicado, sem depender da boa vontade de quem está em falta.
    // -----------------------------------------------------------------------
    require!(
        ctx.accounts.vault.resgates_em_atraso == 0,
        DomError::ResgatesEmAtraso
    );

    let now = Clock::get()?.unix_timestamp;
    // A reserva é percentual do patrimônio, e patrimônio é `supply × NAV`. É
    // leitura de NAV que decide quanto dinheiro pode sair: entra na trava de
    // frescura como as outras (DV11).
    require_nav_fresco(
        ctx.accounts.vault.nav_ts,
        now,
        ctx.accounts.vault.max_nav_staleness,
    )?;

    // -----------------------------------------------------------------------
    // TRAVA 2 — allowlist, pelo **owner** da conta de destino.
    // -----------------------------------------------------------------------
    // Guardar a ATA na lista quebraria a cada mint novo e não sobreviveria a uma
    // conta recriada. Guarda-se a carteira; a conta é conferida contra ela e
    // contra o mint do cofre.
    // -----------------------------------------------------------------------
    let dono = ctx.accounts.destino_usdc.owner;
    require!(dono != Pubkey::default(), DomError::DestinationNotAllowed);
    require!(
        ctx.accounts.vault.deploy_allowlist.contains(&dono),
        DomError::DestinationNotAllowed
    );
    require_keys_eq!(
        ctx.accounts.destino_usdc.mint,
        ctx.accounts.vault.usdc_mint,
        DomError::UnknownUsdcMint
    );

    // -----------------------------------------------------------------------
    // TRAVA 3 — reserva + fila. **A fila tem prioridade sobre a operação.**
    // -----------------------------------------------------------------------
    // O que precisa sobrar no caixa depois da saída: a reserva de 10% do
    // patrimônio (D7).
    //
    // **O passivo congelado saiu do piso junto com a fila D+30.** Ele existia
    // porque o resgate antigo era pago do treasury, então dinheiro já prometido
    // não podia sair como capital. O resgate de capital é pago do
    // `endereco_resgate`, que a mesa abastece de FORA do cofre — o treasury não
    // é fonte dele, e somá-lo aqui congelaria caixa contra uma obrigação que não
    // vai sair daqui.
    //
    // Quem cobre o risco de o resgate não ser honrado não é este piso: é a trava
    // de `resgates_em_atraso` logo acima, que é mais forte — ela não reserva
    // caixa, ela **para o capital de sair**.
    // -----------------------------------------------------------------------
    let patrimonio = usdc_from_shares(ctx.accounts.dom_mint.supply, ctx.accounts.vault.nav)?;
    let reserva = u64::try_from(
        (patrimonio as u128)
            .checked_mul(ctx.accounts.vault.reserve_bps as u128)
            .ok_or(DomError::MathOverflow)?
            .checked_div(BPS_DEN)
            .ok_or(DomError::MathOverflow)?,
    )
    .map_err(|_| error!(DomError::MathOverflow))?;

    let piso = reserva;

    let caixa_depois = ctx
        .accounts
        .treasury
        .amount
        .checked_sub(valor)
        .ok_or(DomError::ReserveViolation)?;
    require!(caixa_depois >= piso, DomError::ReserveViolation);

    // -----------------------------------------------------------------------
    // A saída.
    // -----------------------------------------------------------------------
    let vault_bump = ctx.accounts.vault.bump;
    let vault_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, &[vault_bump]]];
    let decimais = ctx.accounts.usdc_mint.decimals;

    transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.usdc_token_program.key(),
            TransferChecked {
                from: ctx.accounts.treasury.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.destino_usdc.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            vault_seeds,
        ),
        valor,
        decimais,
    )?;

    ctx.accounts.vault.deployed_usdc = ctx
        .accounts
        .vault
        .deployed_usdc
        .checked_add(valor)
        .ok_or(DomError::MathOverflow)?;

    // -----------------------------------------------------------------------
    // D-F2-09 — **o re-deploy fecha a janela de saque de lucro.**
    //
    // O marco é este, e não um cronômetro de N dias, porque é este o fato
    // operacional: o ciclo seguinte começa quando o capital volta a campo, e a
    // janela existe justamente no intervalo em que o cofre está cheio. Timer
    // fixo erraria nos dois sentidos — fecharia com o dinheiro parado, ou
    // deixaria aberto depois de o capital já ter saído.
    //
    // O que sobra sem ser sacado **não some**: continua no preço da cota de
    // quem não sacou. O campo é teto de pagamento, não provisão a liquidar.
    // -----------------------------------------------------------------------
    if ctx.accounts.vault.lucro_sacavel_restante > 0 {
        let nao_sacado = ctx.accounts.vault.lucro_sacavel_restante;
        ctx.accounts.vault.lucro_sacavel_restante = 0;
        emit!(JanelaDeLucroEncerrada {
            nao_sacado,
            timestamp: now,
        });
    }

    emit!(CapitalDeployed {
        valor,
        destino: dono,
        treasury_pos: caixa_depois,
        deployed_usdc: ctx.accounts.vault.deployed_usdc,
        timestamp: now,
    });

    Ok(())
}

/// Devolve USDC para a treasury.
///
/// **Não é privilegiada, e é assim de propósito**: a allowlist existe para
/// controlar por onde o dinheiro **sai**. Entrar é irrestrito — qualquer um pode
/// devolver ao cofre, e exigir proposta para isso só criaria um caminho em que o
/// dinheiro fica preso fora esperando quorum.
///
/// Quem assina é o dono da conta de origem, que é quem está entregando o
/// dinheiro. Não há o que autorizar além disso.
#[derive(Accounts)]
pub struct ReturnCapital<'info> {
    pub origem_authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = usdc_mint @ DomError::UnknownUsdcMint,
        has_one = treasury @ DomError::InvalidTreasury,
    )]
    pub vault: Box<Account<'info, Vault>>,
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, token::mint = usdc_mint, token::authority = origem_authority)]
    pub origem_usdc: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,
    pub usdc_token_program: Interface<'info, TokenInterface>,
}

pub fn handle_return_capital(ctx: Context<ReturnCapital>, valor: u64) -> Result<()> {
    require!(valor > 0, DomError::ZeroDeploy);

    let decimais = ctx.accounts.usdc_mint.decimals;
    transfer_checked(
        CpiContext::new(
            ctx.accounts.usdc_token_program.key(),
            TransferChecked {
                from: ctx.accounts.origem_usdc.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.origem_authority.to_account_info(),
            },
        ),
        valor,
        decimais,
    )?;

    // -----------------------------------------------------------------------
    // `saturating_sub`, não `checked_sub`. **Devolver mais do que saiu é o caso
    // bom**: significa que a operação deu lucro. `deployed_usdc` mede capital em
    // campo **a custo**, então o piso é zero — e o lucro aparece no NAV, que é
    // onde ele tem que aparecer, não neste contador.
    // -----------------------------------------------------------------------
    ctx.accounts.vault.deployed_usdc = ctx.accounts.vault.deployed_usdc.saturating_sub(valor);

    ctx.accounts.treasury.reload()?;

    emit!(CapitalReturned {
        valor,
        origem: ctx.accounts.origem_authority.key(),
        treasury_pos: ctx.accounts.treasury.amount,
        deployed_usdc: ctx.accounts.vault.deployed_usdc,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
