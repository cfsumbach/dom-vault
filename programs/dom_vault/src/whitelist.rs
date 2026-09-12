use {
    crate::{constants::WHITELIST_SEED, error::DomError, state::WhitelistEntry},
    anchor_lang::prelude::*,
};

/// Whitelist fail-closed: endereço, dono, discriminador, carteira e flag.
///
/// Um caminho só, usado pelo hook (`execute`) e pelo `deposit`. Duplicar essa
/// lógica seria a forma mais provável de as duas divergirem — e a D5 não
/// distingue: whitelist é whitelist em qualquer ponto de entrada.
///
/// Qualquer uma das frentes falhando é rejeição. Em especial: conta não
/// inicializada tem `owner` do System Program e dados vazios — cai fora nas
/// duas primeiras checagens, nunca é lida como "neutra".
///
/// É de propósito que a conta chegue como `UncheckedAccount`: tipar como
/// `Account<WhitelistEntry>` devolveria `AccountNotInitialized` genérico e
/// misturaria "não existe" com "conta corrompida".
/// Tamanho da entrada ANTES do Upgrade G: `8 + 32 + 1 + 1`.
const TAMANHO_ANTIGO: usize = 42;

pub fn require_whitelisted(info: &AccountInfo, wallet: &Pubkey) -> Result<u64> {
    let (expected, _bump) =
        Pubkey::find_program_address(&[WHITELIST_SEED, wallet.as_ref()], &crate::ID);
    require_keys_eq!(*info.key, expected, DomError::InvalidWhitelistAddress);
    require_keys_eq!(*info.owner, crate::ID, DomError::NotWhitelisted);

    let data = info.try_borrow_data()?;

    // -----------------------------------------------------------------------
    // ⚠️ LE' OS DOIS TAMANHOS, E ISSO NAO E' GENTILEZA — E' O QUE EVITA UM
    //    APAGAO NO SEGUNDO EM QUE O UPGRADE G SUBIR.
    // -----------------------------------------------------------------------
    // O `WhitelistEntry` cresceu de 42 para 50 bytes. As entradas que ja'
    // existem tem 42, e `try_deserialize` no struct NOVO falharia nelas com
    // "unexpected end of input".
    //
    // Esta funcao e' a mesma do `deposit` E do hook de transferencia. Se ela
    // falhasse nas entradas antigas, no instante do upgrade **todo aporte e toda
    // transferencia de cota parariam** — e so' voltariam depois de uma migracao
    // conta a conta, com o fundo fechado no meio.
    //
    // Lendo o prefixo fixo e o rabo SO' SE ELE EXISTIR, a entrada velha continua
    // valendo e significa "sem excecao" — que e' exatamente o que ela quer dizer.
    // Ela cresce quando a mesa a tocar (`update_whitelist`), nao antes.
    //
    // **Nao ha' migracao, e nao ha' janela.**
    // -----------------------------------------------------------------------
    // Le' o PREFIXO a mao, e nao por `try_deserialize`: Borsh recusa um buffer
    // curto para o struct novo, entao as duas tentativas falhariam na entrada de
    // 42 bytes — que e' justamente a que precisa continuar valendo.
    //
    //   0..8   discriminador
    //   8..40  owner
    //   40     active
    //   41     bump
    //   42..50 min_deposit_proprio  ← so' existe depois do Upgrade G
    require!(data.len() >= TAMANHO_ANTIGO, DomError::NotWhitelisted);
    require!(
        data[..8] == *WhitelistEntry::DISCRIMINATOR,
        DomError::NotWhitelisted
    );

    let owner = Pubkey::try_from(&data[8..40]).map_err(|_| error!(DomError::NotWhitelisted))?;
    require_keys_eq!(owner, *wallet, DomError::NotWhitelisted);
    require!(data[40] != 0, DomError::NotWhitelisted);

    // O rabo so' e' lido se a conta ja' cresceu. Entrada antiga devolve 0, que
    // o `deposit` le' como "usa o piso do cofre".
    let proprio = if data.len() >= TAMANHO_ANTIGO + 8 {
        u64::from_le_bytes(
            data[TAMANHO_ANTIGO..TAMANHO_ANTIGO + 8]
                .try_into()
                .map_err(|_| error!(DomError::NotWhitelisted))?,
        )
    } else {
        0
    };
    Ok(proprio)
}
