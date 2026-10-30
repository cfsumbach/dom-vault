use {
    crate::{constants::*, error::DomError, events::PauseToggled, state::Vault},
    anchor_lang::prelude::*,
};

/// `pause` / `unpause`.
///
/// Bloqueia **entrada e pedido novo** — `deposit`, `request_redeem`,
/// `accrue_performance`, `redeem_fee_share`.
///
/// ⚠️ **A frase que estava aqui envelheceu, e a revisão adversária de 13/10 UTC
/// pegou:** ela dizia *"não bloqueia `process_redemptions`: a fila que já existe
/// continua sendo honrada"*. O `process_redemptions` **não existe mais** — a fila
/// D+30 inteira saiu no lote pré-auditoria, e quem paga hoje é o
/// `efetivar_resgate_capital`. E esse **exige NAV fresco**
/// (`resgate_capital.rs:535`, `MAX_NAV_STALENESS` = 26 h): então a fila segue
/// honrada **enquanto houver NAV fresco**, e 26 h depois de uma pausa ela só anda
/// se a mesa publicar pela válvula, que é proposta. O próprio
/// `resgate_capital.rs:532-534` já dizia isso; esta linha é que não.
///
/// A filosofia continua valendo — *quem já pediu resgate antes da crise não pode
/// ser punido por ela* —, mas ela é sustentada pela válvula da mesa, não por
/// ausência de trava.
///
/// O racional está no R1 do runbook: pausa é o análogo on-chain da desmontagem.
/// Quem já pediu resgate antes da crise não pode ser punido por ela.
#[derive(Accounts)]
pub struct SetPause<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
}

pub fn handle_pause(ctx: Context<SetPause>) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    require!(!vault.paused, DomError::Paused);
    vault.paused = true;

    emit!(PauseToggled {
        paused: true,
        authority: vault.authority,
    });
    Ok(())
}

pub fn handle_unpause(ctx: Context<SetPause>) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    require!(vault.paused, DomError::NotPaused);
    vault.paused = false;

    emit!(PauseToggled {
        paused: false,
        authority: vault.authority,
    });
    Ok(())
}

// ═════════════════════════════════════════════════════════════════════════════
// ITEM 3 DO UPGRADE N · PAUSA RÁPIDA
// ═════════════════════════════════════════════════════════════════════════════
//
// Ordem do dono: *"Qualquer um dos 3 membros que votam pausa o cofre sozinho. O
// quarto membro (só propõe e executa) não pausa."*
//
// O `pause` acima exige a `authority` — o vault do Squads —, e isso significa
// **proposta, 2 de 4 votos e execução**. Numa emergência, isso é tempo que não
// existe. A pausa rápida corta o caminho: **um membro votante, uma assinatura.**
//
// ⚠️ E ELA É DELIBERADAMENTE ASSIMÉTRICA: pausar fica fácil, **despausar continua
// exigindo proposta** (o `unpause` acima, inalterado). Parar o cofre errado custa
// uma janela de aporte; deixá-lo correndo numa crise custa o fundo. A assimetria é
// o desenho, não um esquecimento.
//
// ───────────────────────────── DE ONDE VEM A LISTA DE QUEM VOTA
//
// Da conta `Multisig` do Squads, lida na hora. A ordem do dono dá os dois motivos
// de não gravar a lista no cofre, e os dois foram medidos:
//
//   1 o `Vault` não tem byte livre, e crescer a conta em mainnet obriga a
//     realocar — até a migração rodar, TODA instrução falha a desserialização
//     (a mesma razão que pôs o porteiro da whitelist em conta própria)
//   2 lista gravada desatualiza: trocar um membro no Squads não avisaria o cofre,
//     e a pausa ficaria na mão de quem já saiu
//
// ───────────────────────────── ⚠️ O RISCO DO ITEM, E A MITIGAÇÃO
//
// Ler a conta de outro programa **acopla este contrato ao layout do Squads**. Se
// eles mudarem a conta `Multisig` num upgrade deles, ler por offset devolve **lixo
// que parece permissão** — e lixo que parece permissão, numa instrução de pausa,
// é pior que não ter pausa rápida.
//
// As conferências, nesta ordem, antes de olhar um membro:
//
//   1 a DERIVAÇÃO: este multisig gera a `vault.authority`? (`Unauthorized`)
//   2 o DONO da conta é o Squads (`SQUADS_PROGRAM_ID`)
//   3 o DISCRIMINADOR é o medido (`SQUADS_MULTISIG_DISCRIMINATOR`)
//   4 a `Option<Pubkey>` do `rent_collector` tem tag 0 ou 1 — e é ela que move o
//     início do `Vec<Member>`: ⚠️ **com `Some`, tudo desloca 32 bytes**
//   5 o `Vec<Member>` CABE nos bytes da conta
//
// Da 2 em diante, falhando: `LayoutDoSquadsMudou`, e o cofre não pausa por esta
// porta (o `pause` por proposta continua lá).
//
// ───────────────────────────── ⚠️ A PREMISSA QUE ESTA PORTA CARREGA
//
// A lista de membros só é fronteira de segurança enquanto o multisig for
// **autônomo**. Medido hoje na conta do fundo: `config_authority` (bytes 40..72)
// é **toda zero** — autônomo, e a lista só muda por proposta.
//
// ⚠️ Com um `config_authority` não-zero ("controlled multisig" no Squads), as
// instruções de config passam a exigir **só a assinatura dessa chave**: membros,
// quórum e permissões são reescritos **sem voto nenhum**. A derivação continuaria
// passando e esta função continuaria "correta", mas a trava desceria de *três
// votantes* para *uma chave de config*.
//
// **Este contrato NÃO confere o `config_authority`, e é escolha declarada.** A
// ordem pediu conferência de LAYOUT, e pôr config no meio faria a pausa rápida
// parar de funcionar, calada, no dia em que a mesa decidisse — legitimamente —
// usar uma autoridade de config. Fica como premissa escrita, e a decisão de
// travá-la é do dono. ⚠️ Note que trocar o `config_authority` hoje **já exige
// proposta**, então isto não é porta para terceiro: é mudança de modelo de
// confiança por ato da própria mesa.
//
// ───────────────────────────── ⚠️ E O QUE O PREÇO DA PAUSA PASSA A SER
//
// Antes: pausar custava uma proposta — **2 votos de 4**. Agora custa **1
// assinatura de 1 dos 3 votantes**. Isso é o que a ordem pede, e o motivo é bom
// (emergência não espera voto). Mas o custo de SAIR de uma pausa indevida não
// mudou, e ele é maior do que parece — medido no código:
//
//   · `publish_nav` recusa pausado no caminho do ORÁCULO (`publish_nav.rs:102`)
//   · `efetivar_resgate_capital` exige NAV fresco (`resgate_capital.rs:535`),
//     e `MAX_NAV_STALENESS` nasce em 26 h (`constants.rs:148`)
//   · logo: 26 h depois da pausa, **pagar a fila morre com `NavStale`** a menos
//     que a mesa publique pela válvula — proposta, 2 votos, a cada 26 h
//   · e `marcar_vencido` é permissionless e não olha a pausa: pedido que estoura
//     o prazo vira `resgates_em_atraso`, que trava o `deploy_capital`
//
// O remédio de uma pausa indevida repetida não está neste programa: é proposta de
// config no Squads para tirar o membro. **Fica declarado, e não codificado:**
// carência de re-pausa ou "quem pausou não re-pausa" seria comportamento que a
// ordem não pediu.
//
// ───────────────────────────── O LAYOUT, MEDIDO NA MAINNET EM 13/10 UTC
//                              (12/10 23h BRT — a casa datilografa em UTC)
//
// Lendo a conta do multisig `23EM…Lneg` do fundo (264 bytes):
//
// ```text
//   0..8     discriminador ............ e0 74 79 ba 44 a1 4f ec
//   8..40    create_key ............... 39dF…VZdd
//  40..72    config_authority ......... 1111…1111  (zeros = sem autoridade de config)
//  72..74    threshold  u16 ........... 2
//  74..78    time_lock  u32 ........... 0
//  78..86    transaction_index  u64
//  86..94    stale_transaction_index  u64
//     94     rent_collector: tag da Option (0 = None)   <- ⚠️ move o resto
//     95     bump
//  96..100   members: tamanho do Vec  u32 ... 4
//  100..     members: 33 bytes cada (32 da chave + 1 da mascara)
//
//      2PHj…V3nu  mask 7  Initiate+Vote+Execute
//      G5nd…nwt5  mask 5  Initiate+Execute        <- NAO vota, NAO pausa
//      GctM…gi2h  mask 7  Initiate+Vote+Execute
//      HYre…vKa1  mask 7  Initiate+Vote+Execute
// ```
//
// ───────────────────────────── E POR QUE NÃO PRECISA DE CAMPO NOVO
//
// O contrato não guarda o endereço do multisig — ele **confere** que o multisig
// passado é o que gera a própria autoridade dele:
//
// ```text
//   ["multisig", <multisig>, "vault", 0]  sob o Squads  ==  vault.authority
//   medido: dcFs…nXvu, bump 255
// ```
//
// Sem isso, qualquer um criaria um multisig próprio, se poria como membro votante
// e pausaria o cofre do fundo. ⚠️ **A conferência É a trava de autorização**, não
// uma formalidade.

/// Contas da pausa rápida: um membro votante assina, e a conta do multisig entra
/// só para leitura.
#[derive(Accounts)]
pub struct PausaRapida<'info> {
    /// ⚠️ NÃO é a `authority` do cofre: é a carteira de um membro do multisig. A
    /// autorização não vem de `has_one`, vem da leitura do multisig abaixo.
    pub membro: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
    )]
    pub vault: Account<'info, Vault>,
    /// A conta `Multisig` do Squads. ⚠️ Sem tipo Anchor de propósito — ela é de
    /// outro programa, e o que valida não é a desserialização do Anchor, são as
    /// quatro conferências de `membro_vota`.
    /// CHECK: validada em `membro_vota` (dono · discriminador · layout · derivação).
    pub multisig: UncheckedAccount<'info>,
}

/// Confere a conta do Squads e diz se `quem` é membro COM voto.
///
/// Devolve `Ok(true)` só quando as quatro conferências passam **e** a chave está na
/// lista com o bit de voto. `Ok(false)` quando o layout confere mas a chave não
/// vota — e aí quem decide a mensagem de erro é o chamador.
fn membro_vota(
    multisig: &AccountInfo,
    autoridade_do_cofre: &Pubkey,
    quem: &Pubkey,
) -> Result<bool> {
    // ─────────────────────────────────────────────────────────────────────────
    // 1 · A DERIVAÇÃO, E ELA VEM PRIMEIRO
    // ─────────────────────────────────────────────────────────────────────────
    // Este multisig é o que GERA a autoridade deste cofre? ⚠️ **É esta a trava de
    // autorização.** Sem ela, qualquer um cria um multisig próprio, se põe como
    // membro votante e pausa o cofre do fundo.
    //
    // ⚠️ E ELA VEM ANTES DAS OUTRAS TRÊS DE PROPÓSITO, o que foi achado pela
    // revisão adversária de 13/10 UTC: com o dono e o discriminador conferidos
    // primeiro, **qualquer um na rede mandava uma `pausa_rapida` com uma conta de
    // lixo e recebia `LayoutDoSquadsMudou`** — que é o erro cujo texto afirma *"o
    // Squads mudou o layout num upgrade deles"* e cuja razão de existir é ser o
    // sinal de parar tudo. Um alarme de parar-tudo que qualquer um dispara por uma
    // taxa de transação não é alarme.
    //
    // Nesta ordem, só a conta que DE FATO deriva a autoridade do cofre pode
    // produzir o 6090 — e aí o 6090 significa exatamente o que diz. De graça: a
    // função também deixa de tocar os dados de conta arbitrária.
    //
    // ⚠️ A derivação não lê um byte da conta: ela usa só o ENDEREÇO. É por isso que
    // pode vir antes das conferências de layout.
    let (vault_do_squads, _) = Pubkey::find_program_address(
        &[
            b"multisig",
            multisig.key.as_ref(),
            b"vault",
            &[SQUADS_VAULT_INDEX],
        ],
        &SQUADS_PROGRAM_ID,
    );
    require_keys_eq!(
        vault_do_squads,
        *autoridade_do_cofre,
        DomError::Unauthorized
    );

    // 2 · o dono da conta é o Squads
    require_keys_eq!(
        *multisig.owner,
        SQUADS_PROGRAM_ID,
        DomError::LayoutDoSquadsMudou
    );

    let dados = multisig.try_borrow_data()?;

    // 3 · o discriminador é o medido
    require!(dados.len() >= 96, DomError::LayoutDoSquadsMudou);
    require!(
        dados[..8] == SQUADS_MULTISIG_DISCRIMINATOR,
        DomError::LayoutDoSquadsMudou
    );

    // 4 · a Option<Pubkey> do rent_collector, que MOVE o inicio do Vec<Member>
    let inicio_membros = match dados[94] {
        0 => 94 + 1 + 1,      // tag None + bump
        1 => 94 + 1 + 32 + 1, // tag Some + a chave + bump
        _ => return err!(DomError::LayoutDoSquadsMudou),
    };

    // 5 · o Vec<Member> cabe nos bytes da conta
    require!(
        dados.len() >= inicio_membros + 4,
        DomError::LayoutDoSquadsMudou
    );
    let quantos = u32::from_le_bytes(
        dados[inicio_membros..inicio_membros + 4]
            .try_into()
            .map_err(|_| error!(DomError::LayoutDoSquadsMudou))?,
    ) as usize;
    let fim = inicio_membros
        .checked_add(4)
        .and_then(|x| quantos.checked_mul(33).and_then(|y| x.checked_add(y)))
        .ok_or_else(|| error!(DomError::LayoutDoSquadsMudou))?;
    require!(dados.len() >= fim, DomError::LayoutDoSquadsMudou);

    // 6 · a chave na lista, com o bit de voto
    for i in 0..quantos {
        let base = inicio_membros + 4 + i * 33;
        if dados[base..base + 32] == quem.to_bytes() {
            return Ok(dados[base + 32] & SQUADS_PERMISSAO_VOTO != 0);
        }
    }
    Ok(false)
}

pub fn handle_pausa_rapida(ctx: Context<PausaRapida>) -> Result<()> {
    let autoridade = ctx.accounts.vault.authority;
    require!(
        membro_vota(
            &ctx.accounts.multisig.to_account_info(),
            &autoridade,
            ctx.accounts.membro.key
        )?,
        DomError::Unauthorized
    );

    let vault = &mut ctx.accounts.vault;
    require!(!vault.paused, DomError::Paused);
    vault.paused = true;

    // ⚠️ O EVENTO É O MESMO `PauseToggled`, E O `authority` CARREGA O MEMBRO QUE
    // PAUSOU, não a autoridade do cofre.
    //
    // Evento desta casa se decodifica POR ORDEM: campo novo quebraria o backend, e
    // evento novo seria comportamento que a ordem não pediu. Então o campo diz quem
    // autorizou — que nesta porta é o membro, e é a informação que a mesa precisa.
    // ⚠️ Fica DECLARADO: o leitor que supõe "authority == vault.authority" neste
    // evento passa a ver a chave de um membro. Nenhum leitor de hoje supõe isso
    // (o backend lê o campo `paused`), e a decisão de publicar a distinção num campo
    // próprio é do dono.
    emit!(PauseToggled {
        paused: true,
        authority: *ctx.accounts.membro.key,
    });
    Ok(())
}
