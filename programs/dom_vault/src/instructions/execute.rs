use {
    crate::{constants::*, error::DomError, state::*, whitelist::require_whitelisted},
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{
        spl_token_2022::{
            extension::{
                transfer_hook::TransferHookAccount, BaseStateWithExtensions, StateWithExtensions,
            },
            state::Account as Token2022Account,
        },
        Mint, TokenAccount,
    },
};

/// `Execute` da transfer-hook-interface.
///
/// Ordem das contas fixada pela interface (0..3) e pelo que foi gravado em
/// `initialize_extra_account_meta_list` (5..7). Mudar a ordem aqui sem mudar
/// lá quebra a resolução do Token-2022.
#[derive(Accounts)]
pub struct ExecuteHook<'info> {
    pub source: InterfaceAccount<'info, TokenAccount>,
    pub mint: InterfaceAccount<'info, Mint>,
    pub destination: InterfaceAccount<'info, TokenAccount>,
    /// CHECK: autoridade da origem. Quem confere assinatura é o Token-2022,
    /// antes de chamar o hook; aqui a conta é só contexto.
    pub owner: UncheckedAccount<'info>,
    /// CHECK: conta de validação da interface. Só o endereço importa.
    #[account(
        seeds = [EXTRA_ACCOUNT_METAS_SEED, mint.key().as_ref()],
        bump,
    )]
    pub extra_account_meta_list: UncheckedAccount<'info>,
    #[account(seeds = [VAULT_SEED], bump = vault.bump)]
    pub vault: Account<'info, Vault>,
    /// CHECK: whitelist da origem. Validada no handler — tipar como `Account`
    /// devolveria `AccountNotInitialized` genérico quando a conta não existe, e
    /// o caso "conta ausente" é justamente o que precisa de erro próprio (T22).
    pub source_whitelist: UncheckedAccount<'info>,
    /// CHECK: whitelist do destino. Idem.
    pub destination_whitelist: UncheckedAccount<'info>,
}

pub fn handle_execute(ctx: Context<ExecuteHook>, _amount: u64) -> Result<()> {
    let vault = &ctx.accounts.vault;

    // ---------------------------------------------------------------------
    // D5.1 — o mint é o do DOM. Primeira checagem, antes de qualquer leitura
    // de estado: sem ela, qualquer um cria um mint próprio, aponta o hook para
    // este programa e invoca `Execute` com contas escolhidas a dedo (T19).
    // ---------------------------------------------------------------------
    require_keys_eq!(
        ctx.accounts.mint.key(),
        vault.dom_mint,
        DomError::UnknownMint
    );

    // ---------------------------------------------------------------------
    // D5.2 — origem e destino pertencem a esse mint (T20).
    // ---------------------------------------------------------------------
    require_keys_eq!(
        ctx.accounts.source.mint,
        vault.dom_mint,
        DomError::TokenAccountMintMismatch
    );
    require_keys_eq!(
        ctx.accounts.destination.mint,
        vault.dom_mint,
        DomError::TokenAccountMintMismatch
    );

    // ---------------------------------------------------------------------
    // D5.3 — `transferring` nas duas contas. O Token-2022 liga essa flag antes
    // do CPI e desliga depois; fora de uma transferência real ela está `false`.
    // É o que bloqueia invocação direta do `Execute` (T21).
    // ---------------------------------------------------------------------
    require!(
        is_transferring(&ctx.accounts.source.to_account_info())?,
        DomError::NotTransferring
    );
    require!(
        is_transferring(&ctx.accounts.destination.to_account_info())?,
        DomError::NotTransferring
    );

    // ---------------------------------------------------------------------
    // D5.4 — whitelist de origem e destino, presente e ativa. Conta ausente é
    // rejeição, nunca "passa por omissão" (T22).
    //
    // Isenção: contas cuja autoridade é o PDA de escrow do programa (D2/T24).
    // Não são carteiras de cotista — são o próprio cofre movendo cotas em
    // `request_redeem`/`process_redemptions`, e não há whitelist a emitir para
    // um PDA.
    // ---------------------------------------------------------------------
    let source_owner = ctx.accounts.source.owner;
    let destination_owner = ctx.accounts.destination.owner;
    let source_exempt = source_owner == vault.escrow_authority;
    let destination_exempt = destination_owner == vault.escrow_authority;

    if !source_exempt {
        require_whitelisted(&ctx.accounts.source_whitelist, &source_owner)?;
    }
    if !destination_exempt {
        require_whitelisted(&ctx.accounts.destination_whitelist, &destination_owner)?;
    }

    // ---------------------------------------------------------------------
    // D3 — cap de 25% do supply, se ativo. Contas do programa são isentas (T24).
    //
    // D4: o Token-2022 já gravou `source.amount -= x` e `destination.amount += x`
    // **antes** deste CPI (confirmado em `token-2022/processor.rs`). Portanto lê-se
    // `destination.amount` direto contra `mint.supply`. Somar `amount` à mão aqui
    // seria double-count.
    //
    // Estritamente maior é que rejeita: 25,000000% exato passa (T09).
    // ---------------------------------------------------------------------
    // ---------------------------------------------------------------------
    // D-F2-09 — **transferência de cota fechada enquanto a janela de saque de
    // lucro está aberta.**
    //
    // É a trava irmã da do `deposit`, e sem ela a marca por carteira não vale
    // nada: quem já sacou passa as cotas para uma carteira virgem, que não tem
    // marca, e saca de novo o mesmo lucro. A marca é por dono; a cota não é.
    //
    // **Destino isento é o escrow do programa.** Cota entrando no escrow é
    // `request_redeem`, e ela sai do saldo de quem pediu — reduz o direito
    // dele, nunca cria direito para ninguém. Manter esse caminho aberto é o que
    // impede a janela de trancar o pedido de resgate, que é direito do cotista
    // (D-F2-03). Cota **saindo** do escrow não existe: não há cancelamento, e o
    // pagamento é queima, que não passa por aqui (D17).
    // ---------------------------------------------------------------------
    if vault.lucro_sacavel_restante > 0 && !destination_exempt {
        return err!(DomError::JanelaDeLucroAberta);
    }

    if vault.cap_enforced && !destination_exempt {
        let balance = ctx.accounts.destination.amount as u128;
        let supply = ctx.accounts.mint.supply as u128;
        require!(
            balance.saturating_mul(100)
                <= supply.saturating_mul(ctx.accounts.vault.cap_pct as u128),
            DomError::CapExceeded
        );
    }

    Ok(())
}

/// Lê a flag `transferring` da extensão `TransferHookAccount`.
///
/// Extensão ausente ou conta ilegível = `false` por erro, não por omissão.
fn is_transferring(info: &AccountInfo) -> Result<bool> {
    let data = info.try_borrow_data()?;
    let state = StateWithExtensions::<Token2022Account>::unpack(&data)
        .map_err(|_| error!(DomError::NotTransferring))?;
    let extension = state
        .get_extension::<TransferHookAccount>()
        .map_err(|_| error!(DomError::NotTransferring))?;
    Ok(bool::from(extension.transferring))
}
