use {
    crate::{constants::*, error::DomError, state::Vault},
    anchor_lang::{prelude::*, system_program},
    anchor_spl::token_interface::Mint,
    spl_tlv_account_resolution::{
        account::ExtraAccountMeta, seeds::Seed, state::ExtraAccountMetaList,
    },
    spl_transfer_hook_interface::{
        get_extra_account_metas_address, instruction::ExecuteInstruction,
    },
};

/// Índices das contas na instrução `Execute` da interface, usados para resolver
/// as contas extras. Fixos pela transfer-hook-interface:
///
/// | 0 | source | 1 | mint | 2 | destination | 3 | authority | 4 | validação |
///
/// As extras entram a partir de 5, **na ordem em que são gravadas aqui** —
/// mesma ordem do `ExecuteHook`.
const IX_SOURCE: u8 = 0;
const IX_DESTINATION: u8 = 2;

/// Offset do campo `owner` dentro de uma conta de token SPL: `mint` (32 bytes)
/// vem primeiro. É daí que sai a carteira usada como seed da whitelist.
const TOKEN_ACCOUNT_OWNER_OFFSET: u8 = 32;
const PUBKEY_LEN: u8 = 32;

#[derive(Accounts)]
pub struct InitializeExtraAccountMetaList<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub authority: Signer<'info>,
    #[account(
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
        constraint = vault.dom_mint == mint.key() @ DomError::UnknownMint,
    )]
    pub vault: Account<'info, Vault>,
    pub mint: InterfaceAccount<'info, Mint>,
    /// CHECK: PDA de validação da interface. Endereço conferido pelas seeds e,
    /// de novo, contra `get_extra_account_metas_address` no handler. Criado
    /// aqui, então não dá para tipar como `Account`.
    #[account(
        mut,
        seeds = [EXTRA_ACCOUNT_METAS_SEED, mint.key().as_ref()],
        bump,
    )]
    pub extra_account_meta_list: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

/// Contas extras que o Token-2022 resolve e entrega ao hook a cada
/// transferência. A resolução é feita pelo próprio Token-2022 a partir daqui —
/// o cliente não escolhe quais contas manda, só as deriva.
fn extra_account_metas() -> Result<Vec<ExtraAccountMeta>> {
    Ok(vec![
        // 5 — estado do cofre: mint do DOM, cap_enforced, escrow.
        ExtraAccountMeta::new_with_seeds(
            &[Seed::Literal {
                bytes: VAULT_SEED.to_vec(),
            }],
            false,
            false,
        )?,
        // 6 — whitelist da carteira de origem.
        ExtraAccountMeta::new_with_seeds(
            &[
                Seed::Literal {
                    bytes: WHITELIST_SEED.to_vec(),
                },
                Seed::AccountData {
                    account_index: IX_SOURCE,
                    data_index: TOKEN_ACCOUNT_OWNER_OFFSET,
                    length: PUBKEY_LEN,
                },
            ],
            false,
            false,
        )?,
        // 7 — whitelist da carteira de destino.
        ExtraAccountMeta::new_with_seeds(
            &[
                Seed::Literal {
                    bytes: WHITELIST_SEED.to_vec(),
                },
                Seed::AccountData {
                    account_index: IX_DESTINATION,
                    data_index: TOKEN_ACCOUNT_OWNER_OFFSET,
                    length: PUBKEY_LEN,
                },
            ],
            false,
            false,
        )?,
    ])
}

pub fn handle_initialize_extra_account_meta_list(
    ctx: Context<InitializeExtraAccountMetaList>,
) -> Result<()> {
    let mint_key = ctx.accounts.mint.key();
    let list_info = ctx.accounts.extra_account_meta_list.to_account_info();

    // O endereço já saiu das seeds do Anchor; conferir de novo contra o crate da
    // interface custa nada e trava a divergência caso o literal da seed mude do
    // lado do SPL.
    require_keys_eq!(
        *list_info.key,
        get_extra_account_metas_address(&mint_key, &crate::ID),
        DomError::InvalidExtraAccountMetaListAddress
    );

    let metas = extra_account_metas()?;
    let space = ExtraAccountMetaList::size_of(metas.len())?;
    let lamports = Rent::get()?.minimum_balance(space);

    let bump = [ctx.bumps.extra_account_meta_list];
    let signer_seeds: &[&[&[u8]]] = &[&[EXTRA_ACCOUNT_METAS_SEED, mint_key.as_ref(), &bump]];

    system_program::create_account(
        CpiContext::new_with_signer(
            system_program::ID,
            system_program::CreateAccount {
                from: ctx.accounts.payer.to_account_info(),
                to: list_info.clone(),
            },
            signer_seeds,
        ),
        lamports,
        space as u64,
        &crate::ID,
    )?;

    let mut data = list_info.try_borrow_mut_data()?;
    ExtraAccountMetaList::init::<ExecuteInstruction>(&mut data, &metas)?;

    Ok(())
}
