use {
    crate::{
        constants::*, error::DomError, events::NavPublished, math::nav_dentro_do_bound,
        state::Vault,
    },
    anchor_lang::prelude::*,
};

/// Publica o NAV do fundo, com hash de atestação e timestamp.
///
/// Em F1 o oráculo é uma chave operada por script; o backend de verdade é F3
/// (R5 do runbook). O que já é definitivo aqui é o formato: **NAV sem hash de
/// atestação não entra**, e o relógio não anda para trás.
///
/// # DV11 — dois caminhos, e é isso que torna o limite seguro
///
/// O **oráculo** publica dentro do limite de variação (`NAV_BOUND`, 15%). Fora
/// dele é recusado: é esta trava que para o vetor do achado — oráculo
/// comprometido publicando `NAV = 0,000001` para mintar cota infinita no
/// depósito seguinte.
///
/// A **autoridade** — o Vault PDA do Squads, portanto uma proposta com quorum —
/// publica **qualquer** valor. Sem essa válvula, um dia de queda real de 20%
/// seria recusado, o NAV pararia de ser publicado, 26h depois o `staleness`
/// derrubaria depósito, pedido, fila e apuração, e o cofre travaria exatamente
/// na crise. A válvula é o que separa "limite" de "cofre bricado".
///
/// O vetor continua barrado porque o atacante que tem a chave do oráculo **não
/// tem o multisig**.
#[derive(Accounts)]
pub struct PublishNav<'info> {
    /// O oráculo **ou** a autoridade. Qual dos dois é conferido no handler,
    /// porque a diferença não é de acesso — é de quanto pode variar.
    pub publisher: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
    )]
    pub vault: Account<'info, Vault>,
}

pub fn handle_publish_nav(
    ctx: Context<PublishNav>,
    nav: u64,
    attestation_hash: [u8; 32],
    timestamp: i64,
) -> Result<()> {
    require!(nav > 0, DomError::InvalidNav);

    let quem = ctx.accounts.publisher.key();
    let vault = &mut ctx.accounts.vault;

    let pelo_oraculo = quem == vault.nav_oracle;
    let pela_mesa = quem == vault.authority;
    require!(pelo_oraculo || pela_mesa, DomError::NotOracle); // T35

    // Monotônico: NAV regressivo reabriria a janela do "pior valor" para trás e
    // permitiria escolher o instante mais conveniente (T36).
    require!(timestamp > vault.nav_ts, DomError::NavTimestampRegressive);

    // E não pode vir do futuro: o D+30 e o ciclo de cap andam pelo relógio da
    // rede, então um NAV pós-datado antecipa elegibilidade que não existe.
    let now = Clock::get()?.unix_timestamp;
    require!(timestamp <= now, DomError::NavTimestampInFuture);

    // -----------------------------------------------------------------------
    // DV11 — o limite de variação, e só para o oráculo.
    // -----------------------------------------------------------------------
    // A mesa atravessa por proposta. O evento carrega qual caminho foi usado:
    // NAV publicado fora do limite é fato que a transparência tem que mostrar,
    // não detalhe de implementação.
    // -----------------------------------------------------------------------
    let nav_anterior = vault.nav;
    if pelo_oraculo {
        require!(
            nav_dentro_do_bound(nav_anterior, nav, vault.nav_bound_pct),
            DomError::NavOutOfBounds
        );

        // Cofre pausado fecha o caminho de rotina. **A válvula da mesa segue
        // aberta** — pausar existe para parar o automático, não para impedir a
        // mesa de corrigir um NAV errado, que é o motivo mais provável da pausa.
        //
        // **Antes do intervalo, de propósito:** se as duas se aplicam, "o cofre
        // está pausado" é o diagnóstico útil. "Cedo demais" mandaria o operador
        // esperar uma hora para descobrir a mesma coisa.
        require!(!vault.paused, DomError::Paused);

        // Intervalo mínimo: transforma os 15% de limite POR PUBLICAÇÃO em limite
        // de TAXA. Sem ele, cinco publicações em cinco segundos dobram ou
        // reduzem o NAV à metade, e o bound não protege contra quem tem a chave.
        //
        // `nav_ts == 0` é a gênese e passa: não há publicação anterior de que
        // guardar distância.
        if vault.nav_ts > 0 {
            require!(
                now.saturating_sub(vault.nav_ts) >= vault.min_nav_publish_interval,
                DomError::NavPublicacaoMuitoCedo
            );
        }

        // -------------------------------------------------------------------
        // O timestamp do oráculo acompanha o relógio da rede — e é esta trava
        // que faz o intervalo acima valer.
        //
        // O intervalo compara `now` com `nav_ts`, e `nav_ts` é o timestamp que
        // o publicador informou, não o instante da publicação. Sem teto no
        // atraso, uma lacuna de duas horas deixava publicar `nav_ts + 1`,
        // `nav_ts + 2`, … em segundos: cada uma passa no intervalo (`now − nav_ts`
        // segue ≥ 1h), na monotonicidade e no teto de futuro. Cinco passos de
        // 15% em cinco segundos — o cenário exato que o intervalo fecha.
        //
        // O teto é `MAX_NAV_TIMESTAMP_LAG`, ou metade do intervalo vigente se
        // ele for menor: assim duas publicações do oráculo distam pelo menos
        // `intervalo − lag ≥ intervalo / 2` em tempo real, qualquer que seja o
        // intervalo votado. Inclusivo, como os outros limites do cofre.
        // -------------------------------------------------------------------
        let lag_max = MAX_NAV_TIMESTAMP_LAG.min(vault.min_nav_publish_interval / 2);
        require!(
            now.saturating_sub(timestamp) <= lag_max,
            DomError::NavTimestampMuitoAntigo
        );
    }
    // A válvula só "abriu" quando a mesa publicou algo que o oráculo não
    // conseguiria. Proposta dentro do limite é publicação comum.
    let pela_valvula =
        !pelo_oraculo && !nav_dentro_do_bound(nav_anterior, nav, vault.nav_bound_pct);

    vault.nav = nav;
    vault.nav_ts = timestamp;
    // DV12: a atestação fica no estado, não só no evento — evento expira da
    // janela de retenção do RPC e leva junto a prova de qual laudo sustentava
    // qual NAV.
    vault.nav_attestation = attestation_hash;

    emit!(NavPublished {
        nav,
        attestation_hash,
        timestamp,
        publisher: quem,
        nav_anterior,
        pela_valvula,
    });

    Ok(())
}

// ===========================================================================
// Rotação da chave do oráculo — a mesa
// ===========================================================================

/// **A revogação que não existia.**
///
/// `nav_oracle` era escrito num lugar só do programa — o `initialize`. Sem esta
/// instrução, chave de oráculo vazada não tinha revogação: o `pause` não trava o
/// `publish_nav`, a mesa e o atacante publicam os dois, vale a última escrita, e
/// o atacante publica a cada segundo enquanto a mesa precisa de dois votos. A
/// mesa perdia essa corrida, e o único recurso era upgrade de emergência.
///
/// Não muda o struct — escreve um campo que já existia. Foi por isso que coube
/// no lote sem custo próprio de migração.
#[derive(Accounts)]
pub struct SetNavOracle<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
}

pub fn handle_set_nav_oracle(ctx: Context<SetNavOracle>, novo: Pubkey) -> Result<()> {
    require_keys_neq!(novo, Pubkey::default(), DomError::NotOracle);
    // A autoridade não pode ser o oráculo: seria a válvula da mesa e o caminho de
    // rotina na mesma chave, e o limite de 15% deixaria de significar alguma
    // coisa — quem pode atravessar o limite não deveria poder publicar sozinho.
    require_keys_neq!(novo, ctx.accounts.vault.authority, DomError::NotOracle);
    let anterior = ctx.accounts.vault.nav_oracle;
    ctx.accounts.vault.nav_oracle = novo;
    emit!(crate::events::NavOracleTrocado { anterior, novo });
    Ok(())
}
