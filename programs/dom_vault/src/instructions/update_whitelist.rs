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
    /// A mesa **ou** o porteiro. Qual dos dois e' conferido no handler.
    pub authority: Signer<'info>,
    // -------------------------------------------------------------------------
    // ⚠️ O `has_one = authority` SAIU DAQUI, e foi para o handler — `D-F2-34`.
    // -------------------------------------------------------------------------
    // Ele so' sabe comparar com UM campo, e agora sao dois caminhos: a
    // autoridade da mesa, ou o porteiro nomeado por ela. A conferencia desceu
    // para o handler, onde cabe o "ou" — e onde a recusa diz QUAL dos dois
    // faltou, em vez de um `Unauthorized` mudo.
    #[account(seeds = [VAULT_SEED], bump = vault.bump)]
    pub vault: Account<'info, Vault>,
    /// CHECK: conta do porteiro. Opcional — quando ausente, so' a mesa aprova.
    /// O PDA e' conferido pelas seeds; o conteudo, no handler.
    #[account(seeds = [WL_OPERATOR_SEED], bump)]
    pub wl_operator: UncheckedAccount<'info>,
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
    // -------------------------------------------------------------------------
    // QUEM ASSINOU: a mesa, ou o porteiro — `D-F2-34`.
    // -------------------------------------------------------------------------
    // A conta do porteiro pode nem existir: e' assim que o cofre nasce, e e'
    // assim que ele fica se a mesa nunca nomear ninguem. Conta vazia NAO e'
    // porteiro nulo que autoriza todo mundo — e' porteiro AUSENTE, e ai' so' a
    // mesa aprova, que era o comportamento ate' aqui.
    //
    // ⚠️ A ordem importa: a mesa e' conferida PRIMEIRO. Se um dia a conta do
    // porteiro for corrompida ou ficar ilegivel, a mesa continua entrando —
    // perder a whitelist por causa da conta da delegacao seria trocar um
    // gargalo por uma tranca.
    // -------------------------------------------------------------------------
    let quem = ctx.accounts.authority.key();
    let pela_mesa = quem == ctx.accounts.vault.authority;

    let pelo_porteiro = if pela_mesa {
        false
    } else {
        let conta = ctx.accounts.wl_operator.to_account_info();
        let dados = conta.try_borrow_data()?;
        conta.owner == &crate::ID
            && dados.len() >= 8 + 32
            && dados[..8] == *WhitelistOperator::DISCRIMINATOR
            && Pubkey::try_from(&dados[8..40]).map(|k| k == quem).unwrap_or(false)
            // `Pubkey::default()` e' "desligado", nunca um assinante valido.
            && quem != Pubkey::default()
    };

    require!(pela_mesa || pelo_porteiro, DomError::Unauthorized);

    // -------------------------------------------------------------------------
    // ⚠️ O PORTEIRO INSCREVE. DESINSCREVER CONTINUA SENDO 2/3.
    // -------------------------------------------------------------------------
    // A assimetria nao e' zelo: e' o unico jeito de a delegacao nao ser pior do
    // que o gargalo que ela resolve.
    //
    // Desinscrever TRANCA O DINHEIRO DE OUTRO. Quem sai da whitelist nao
    // transfere cota (o hook barra as duas pontas) e nao pede resgate de capital
    // (`resgate_capital.rs` exige whitelist em `solicitar`). O capital fica
    // imobilizado ate' a mesa readmitir — **por 2/3, que e' exatamente o quorum
    // que a chave do porteiro contorna**.
    //
    // Inscrever tem o limite natural de exigir que a pessoa APORTE dinheiro de
    // verdade para virar cotista. Desinscrever nao tem limite nenhum, e e'
    // unilateral. Sao riscos de ordens diferentes, e so' um deles vale delegar.
    //
    // E a EXCECAO DE PISO tambem fica com a mesa: conceder piso menor e' decisao
    // de politica sobre dinheiro, nao operacao de porta. O porteiro inscreve no
    // piso do cofre, que e' `0` neste campo — "sem excecao".
    // -------------------------------------------------------------------------
    if pelo_porteiro {
        require!(active, DomError::Unauthorized);
        require!(min_deposit_proprio == 0, DomError::Unauthorized);
    }

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
