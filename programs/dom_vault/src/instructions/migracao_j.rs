//! Migração do cofre para o layout do Upgrade J — 603 → 731 bytes.
//!
//! Mesma forma da migração do E (541 → 603): a imagem nova é montada fora da
//! conta, a cabeça é copiada byte a byte, os campos novos entram DEPOIS de
//! `perf_fee_bps_total` e o `layout_version` anda para o fim. Acréscimo puro:
//! nenhum offset existente muda.
//!
//!   campo                  antigo (603)   novo (731)
//!   [cabeça]               0..601         0..601      igual
//!   gaveta_usdc            —              601..633    zero (set_gaveta_usdc na mesma proposta)
//!   indice_p               —              633..649    0
//!   indice_ciclo           —              649..665    0
//!   indice_ciclo_anterior  —              665..681    0
//!   gaveta_saldo_visto     —              681..689    0
//!   p_ciclo                —              689..697    0
//!   nav_piso               —              697..705    nav atual
//!   nav_fechamento         —              705..713    0
//!   indice_diluicao        —              713..729    0
//!   layout_version         601..603       729..731    2 → 3
//!
//! **A gaveta nasce vazia** e a proposta de migração carrega o
//! `set_gaveta_usdc` logo atrás (a ATA do vault 1, `9ecvK773…`). O saldo visto
//! nasce ZERO: o que estiver lá na primeira leitura entra como P do ciclo em
//! curso. `indice_diluicao` nasce zero.
//!
//! **Temporária.** Sai no upgrade seguinte, como a do E saiu no F.
use {
    crate::{constants::*, error::DomError, state::Vault},
    anchor_lang::prelude::*,
};

const TAMANHO_ANTIGO: usize = 603;
const TAMANHO_NOVO: usize = 8 + Vault::INIT_SPACE; // 731
/// Fim de `perf_fee_bps_total` no layout 2.
const FIM_DA_CABECA: usize = 601;
/// Onde `nav` mora (offset conferido em `tests/estado_f2.rs`: 328).
const OFF_NAV: usize = 328;

#[derive(Accounts)]
pub struct MigrarVaultIndice<'info> {
    pub authority: Signer<'info>,
    /// CHECK: `UncheckedAccount` de propósito — a conta viva está no layout
    /// antigo e `Account<Vault>` desserializaria antes das constraints.
    #[account(mut, seeds = [VAULT_SEED], bump)]
    pub vault: UncheckedAccount<'info>,
    #[account(mut)]
    pub pagador: Signer<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handle_migrar_vault_indice(ctx: Context<MigrarVaultIndice>) -> Result<()> {
    let conta = &ctx.accounts.vault;

    let nav_atual = {
        let dados = conta.try_borrow_data()?;
        require!(dados.len() >= 40, DomError::LayoutVersaoInesperada);
        let autoridade_gravada = Pubkey::try_from(&dados[8..40])
            .map_err(|_| error!(DomError::LayoutVersaoInesperada))?;
        require_keys_eq!(
            autoridade_gravada,
            ctx.accounts.authority.key(),
            DomError::Unauthorized
        );
        require!(
            dados.len() == TAMANHO_ANTIGO,
            DomError::LayoutVersaoInesperada
        );
        let versao = u16::from_le_bytes([dados[601], dados[602]]);
        require!(versao == 2, DomError::LayoutVersaoInesperada);
        u64::from_le_bytes(dados[OFF_NAV..OFF_NAV + 8].try_into().unwrap())
    };

    let mut novo = vec![0u8; TAMANHO_NOVO];
    {
        let antigo = conta.try_borrow_data()?;
        novo[..FIM_DA_CABECA].copy_from_slice(&antigo[..FIM_DA_CABECA]);
    }
    let mut p = FIM_DA_CABECA;
    novo[p..p + 32].copy_from_slice(Pubkey::default().as_ref()); // gaveta_usdc: set_gaveta_usdc aponta
    p += 32;
    for _ in 0..3 {
        novo[p..p + 16].copy_from_slice(&0u128.to_le_bytes());
        p += 16;
    }
    novo[p..p + 8].copy_from_slice(&0u64.to_le_bytes()); // gaveta_saldo_visto
    p += 8;
    novo[p..p + 8].copy_from_slice(&0u64.to_le_bytes()); // p_ciclo
    p += 8;
    novo[p..p + 8].copy_from_slice(&nav_atual.to_le_bytes()); // nav_piso nasce no NAV de agora
    p += 8;
    novo[p..p + 8].copy_from_slice(&0u64.to_le_bytes()); // nav_fechamento
    p += 8;
    novo[p..p + 16].copy_from_slice(&0u128.to_le_bytes()); // indice_diluicao
    p += 16;
    novo[p..p + 2].copy_from_slice(&LAYOUT_VERSION.to_le_bytes());
    p += 2;
    require!(p == TAMANHO_NOVO, DomError::LayoutVersaoInesperada);

    let rent = Rent::get()?;
    let minimo = rent.minimum_balance(TAMANHO_NOVO);
    let atual = conta.lamports();
    if minimo > atual {
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.key(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.pagador.to_account_info(),
                    to: conta.to_account_info(),
                },
            ),
            minimo - atual,
        )?;
    }
    conta.resize(TAMANHO_NOVO)?;
    conta.try_borrow_mut_data()?.copy_from_slice(&novo);

    msg!(
        "vault migrado: {} -> {} bytes (layout 3)",
        TAMANHO_ANTIGO,
        TAMANHO_NOVO
    );
    Ok(())
}
