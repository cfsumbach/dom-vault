use {
    crate::{constants::*, error::DomError, events::SociosUpdated, state::Vault},
    anchor_lang::prelude::*,
};

/// Atualiza as três carteiras de sócio.
///
/// **A mesma trava do `initialize`** (D10): distintas par a par. O mandato §1.2
/// diz que os endereços são configuráveis, logo existe este caminho — e sem a
/// checagem aqui a validação do `initialize` seria contornável em dois passos:
/// configura distinto, atualiza para duplicado.
#[derive(Accounts)]
pub struct UpdateSocios<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
    // -----------------------------------------------------------------------
    // ⚠️ DECISOES 13/10 21:10 · TROCAR SÓCIO COM RETRATO ABERTO É PROIBIDO
    // -----------------------------------------------------------------------
    /// O retrato do ciclo que o chamador declara ser o **último**.
    ///
    /// ─── por que esta conta existe ─────────────────────────────────────────
    /// A Fase A debita a custódia da parte dos sócios **de então**, e o lote que
    /// fecha devolve a sobra aos sócios **de agora**. Um `update_socios` entre as
    /// duas coisas manda dinheiro para quem não foi debitado — achado da revisão
    /// adversária de 13/10, e o dono mandou recusar.
    ///
    /// ⚠️ **E ele também destrava um travamento:** `carregar_posicao` não cria
    /// posição, então um sócio novo sem `PosicaoDoCotista` fazia o lote que fecha
    /// recusar com `PosicaoDoCotistaAusente` **para sempre**, com o retrato aberto.
    ///
    /// CHECK: conferido no corpo — PDA, dono, discriminador e se é o último.
    pub retrato: UncheckedAccount<'info>,
    /// O retrato de `ciclo + 1`, que tem de estar **vazio**.
    ///
    /// ⚠️ **É ele que prova que o `retrato` é o ÚLTIMO**, e sem essa prova a
    /// conferência não valeria nada: o cofre não tem contador de ciclo, então
    /// bastaria passar um retrato antigo e fechado para escapar. Com os dois, não
    /// há como: se o ciclo N está aberto e é o último, declarar N recusa (aberto),
    /// declarar N−1 recusa (o seguinte existe) e declarar N+1 recusa (não é o
    /// primeiro).
    ///
    /// CHECK: conferido no corpo — PDA de `ciclo + 1` e vazia.
    pub retrato_seguinte: UncheckedAccount<'info>,
}

/// Distinção par a par. Com três elementos são três comparações — explícito é
/// melhor que laço aqui, porque a lista não cresce (o 20/20/20 pressupõe três).
pub fn require_socios_distintos(socios: &[Pubkey; NUM_SOCIOS]) -> Result<()> {
    require_keys_neq!(socios[0], socios[1], DomError::DuplicateSocio);
    require_keys_neq!(socios[0], socios[2], DomError::DuplicateSocio);
    require_keys_neq!(socios[1], socios[2], DomError::DuplicateSocio);
    Ok(())
}

pub fn handle_update_socios(
    ctx: Context<UpdateSocios>,
    ciclo: u64,
    socios: [Pubkey; NUM_SOCIOS],
) -> Result<()> {
    // -----------------------------------------------------------------------
    // ⚠️ DECISOES 13/10 21:10 · a troca RECUSA com retrato aberto
    // -----------------------------------------------------------------------
    conferir_sem_retrato_aberto(
        &ctx.accounts.retrato,
        &ctx.accounts.retrato_seguinte,
        ciclo,
        ctx.program_id,
    )?;

    require_socios_distintos(&socios)?;

    ctx.accounts.vault.socios = socios;

    emit!(SociosUpdated { socios });
    Ok(())
}

/// ⚠️ DECISOES 13/10 21:10 — recusa a troca de sócio enquanto houver retrato aberto.
///
/// O chamador declara qual ciclo é o último, e as duas contas o provam:
///
/// 1. `retrato` é a PDA de `ciclo` e `retrato_seguinte` a de `ciclo + 1` — as duas
///    conferidas, sempre;
/// 2. se `retrato` tem dados, ele tem de ser do programa, ser um `RetratoDoCiclo`,
///    ter o `ciclo` gravado igual ao declarado, e estar **FECHADO**;
/// 3. e `retrato_seguinte` tem de estar **vazio**, que é o que prova que o `retrato`
///    é o último;
/// 4. `retrato` vazio só é aceito no primeiro ciclo — antes de qualquer fechamento
///    com retrato não há dívida de ciclo para proteger.
fn conferir_sem_retrato_aberto(
    retrato: &UncheckedAccount,
    seguinte: &UncheckedAccount,
    ciclo: u64,
    program_id: &Pubkey,
) -> Result<()> {
    let (pda, _) = Pubkey::find_program_address(&[RETRATO_SEED, &ciclo.to_le_bytes()], program_id);
    require_keys_eq!(retrato.key(), pda, DomError::RetratoDoCicloInvalido);
    let proximo = ciclo
        .checked_add(1)
        .ok_or(error!(DomError::RetratoDoCicloInvalido))?;
    let (pda_seg, _) =
        Pubkey::find_program_address(&[RETRATO_SEED, &proximo.to_le_bytes()], program_id);
    require_keys_eq!(seguinte.key(), pda_seg, DomError::RetratoDoCicloInvalido);
    // ⚠️ `data_is_empty` e nao `lamports`: 1 lamport de qualquer um nao cria dados,
    // e medir existencia por lamport foi o defeito que travava o cofre (Fase A).
    require!(
        seguinte.to_account_info().data_is_empty(),
        DomError::CicloDeclaradoNaoEOUltimo
    );

    let info = retrato.to_account_info();
    if info.data_is_empty() {
        require!(
            ciclo == PRIMEIRO_CICLO_DO_RETRATO,
            DomError::RetratoDoCicloInvalido
        );
        return Ok(());
    }
    require_keys_eq!(*info.owner, *program_id, DomError::RetratoDoCicloInvalido);
    let dados = info.try_borrow_data()?;
    let r = crate::state::RetratoDoCiclo::try_deserialize(&mut &dados[..])
        .map_err(|_| error!(DomError::RetratoDeOutroLayout))?;
    require_eq!(r.ciclo, ciclo, DomError::RetratoDoCicloInvalido);
    require!(!r.aberto, DomError::RetratoDeclaradoAberto);
    Ok(())
}
