use {
    crate::{constants::*, error::DomError, events::WhitelistUpdated, state::*},
    anchor_lang::{prelude::*, system_program, Discriminator},
};

/// Tamanho da entrada ANTES do Upgrade G: `8 + 32 + 1 + 1`.
const TAMANHO_ANTIGO: usize = 42;

#[derive(Accounts)]
#[instruction(owner: Pubkey)]
pub struct UpdateWhitelist<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub authority: Signer<'info>,
    #[account(
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
    // -------------------------------------------------------------------------
    // ⚠️ `UncheckedAccount`, e a razao e' o Upgrade G.
    // -------------------------------------------------------------------------
    // Era `Account<WhitelistEntry>` com `init_if_needed`. Com o campo novo, o
    // struct passou de 42 para 50 bytes — e o `init_if_needed` DESSERIALIZA a
    // conta existente ANTES do handler rodar. Numa entrada de 42 bytes isso
    // falha com `AccountDidNotDeserialize`, e a mesa ficaria sem conseguir
    // tocar justamente as carteiras que ja' aprovou.
    //
    // Aqui a conta e' criada, crescida e escrita a mao. A validacao que se perde
    // e' recuperada no handler: o PDA e' conferido pelas seeds, o dono e'
    // conferido, e o discriminador e' escrito por nos.
    //
    /// CHECK: PDA conferido pelas seeds; conteudo escrito pelo handler.
    #[account(mut, seeds = [WHITELIST_SEED, owner.as_ref()], bump)]
    pub entry: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handle_update_whitelist(
    ctx: Context<UpdateWhitelist>,
    owner: Pubkey,
    active: bool,
    min_deposit_proprio: u64,
) -> Result<()> {
    let info = ctx.accounts.entry.to_account_info();
    let alvo = 8 + WhitelistEntry::INIT_SPACE;
    let bump = ctx.bumps.entry;
    let seeds: &[&[u8]] = &[WHITELIST_SEED, owner.as_ref(), &[bump]];

    if info.data_is_empty() {
        // Primeira aprovacao desta carteira: nasce ja' no tamanho novo.
        system_program::create_account(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.key(),
                system_program::CreateAccount {
                    from: ctx.accounts.payer.to_account_info(),
                    to: info.clone(),
                },
                &[seeds],
            ),
            Rent::get()?.minimum_balance(alvo),
            alvo as u64,
            &crate::ID,
        )?;
    } else if info.data_len() < alvo {
        // ---------------------------------------------------------------------
        // A MIGRACAO E' PREGUICOSA, E NAO TEM JANELA.
        // ---------------------------------------------------------------------
        // Entradas anteriores ao G tem 42 bytes. Redimensionar todas de uma vez
        // exigiria uma instrucao de migracao e um intervalo com aporte e
        // transferencia de cota parados — porque `require_whitelisted` e' a
        // mesma funcao dos dois caminhos.
        //
        // Entao cada entrada cresce quando a mesa a toca. Ate' la' ela le' como
        // "sem excecao", que e' o que ela significa. Ver `whitelist.rs`.
        // ---------------------------------------------------------------------
        require!(info.data_len() >= TAMANHO_ANTIGO, DomError::NotWhitelisted);
        let falta = Rent::get()?
            .minimum_balance(alvo)
            .saturating_sub(info.lamports());
        if falta > 0 {
            system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.key(),
                    system_program::Transfer {
                        from: ctx.accounts.payer.to_account_info(),
                        to: info.clone(),
                    },
                ),
                falta,
            )?;
        }
        info.resize(alvo)?;
    }

    // A escrita e' de todos os campos, sempre. Nao ha' estado parcial herdado —
    // era a mesma garantia que o `init_if_needed` dava, mantida a mao.
    let mut data = info.try_borrow_mut_data()?;
    data[..8].copy_from_slice(WhitelistEntry::DISCRIMINATOR);
    data[8..40].copy_from_slice(owner.as_ref());
    data[40] = active as u8;
    data[41] = bump;
    data[42..50].copy_from_slice(&min_deposit_proprio.to_le_bytes());

    emit!(WhitelistUpdated {
        owner,
        active,
        min_deposit_proprio,
    });
    Ok(())
}
