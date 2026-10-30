use {
    crate::{
        constants::*,
        error::DomError,
        events::{NavPublished, PisoMarcado},
        math::nav_dentro_do_bound,
        state::{RetratoDoCiclo, Vault},
    },
    anchor_lang::prelude::*,
    anchor_spl::token_interface::{Mint, TokenAccount},
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
        has_one = treasury @ DomError::InvalidTreasury,
    )]
    pub vault: Account<'info, Vault>,
    /// Upgrade J: a gaveta. A publicacao absorve o P novo no indice e recalcula
    /// o piso — o oraculo passa a carregar a conta, e so' ela.
    #[account(constraint = gaveta.key() == vault.gaveta_usdc @ DomError::GavetaErrada)]
    pub gaveta: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Upgrade J: o caixa do cofre, para o piso = (campo + caixa + gaveta) ÷ supply.
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Upgrade J: o supply, para o indice e o piso.
    #[account(address = vault.dom_mint @ DomError::UnknownMint)]
    pub dom_mint: Box<InterfaceAccount<'info, Mint>>,
    /// (3a) O retrato do ciclo corrente; ABERTO, o piso não é marcado.
    ///
    /// CHECK: conferido em `piso_livre` — ou é um `RetratoDoCiclo` do programa
    /// na sua PDA e `e_o_corrente`, ou é a PDA VAZIA do primeiro ciclo.
    pub retrato: UncheckedAccount<'info>,
}

/// (3a) O piso pode ser marcado? `false` com o retrato corrente aberto: o piso
/// do ciclo foi fixado na Fase A e não se move até ele fechar (DECISOES 14/10
/// 23:00 item 3; mecanismo 18/10 17:35).
///
/// A conta vazia só vale na PDA de `PRIMEIRO_CICLO_DO_RETRATO`: os retratos
/// formam corrente a partir dela, então ela vazia prova que não há nenhum.
fn piso_livre(retrato: &AccountInfo, vault: &Vault) -> Result<bool> {
    if retrato.owner == &anchor_lang::system_program::ID && retrato.data_is_empty() {
        let (primeiro, _) = Pubkey::find_program_address(
            &[RETRATO_SEED, &PRIMEIRO_CICLO_DO_RETRATO.to_le_bytes()],
            &crate::ID,
        );
        require_keys_eq!(retrato.key(), primeiro, DomError::RetratoNaoEOMaisRecente);
        return Ok(true);
    }
    require_keys_eq!(*retrato.owner, crate::ID, DomError::RetratoDoCicloInvalido);
    // ⚠️ Lido por OFFSET, não por `try_deserialize`: um retrato de outro layout
    // recusaria TODA publicação, e sem NAV fresco nem a Fase A roda para sair.
    // Os campos do começo não se movem (campo novo só no fim, `96500d3`).
    let d = retrato.try_borrow_data()?;
    require!(
        d.len() >= RETRATO_LIDO && d[..8] == *RetratoDoCiclo::DISCRIMINATOR,
        DomError::RetratoDoCicloInvalido
    );
    let u64_em = |i: usize| u64::from_le_bytes(d[i..i + 8].try_into().unwrap());
    let u128_em = |i: usize| u128::from_le_bytes(d[i..i + 16].try_into().unwrap());
    let pda = Pubkey::create_program_address(
        &[RETRATO_SEED, &u64_em(8).to_le_bytes(), &[d[93]]],
        &crate::ID,
    )
    .map_err(|_| error!(DomError::RetratoDoCicloInvalido))?;
    require_keys_eq!(retrato.key(), pda, DomError::RetratoDoCicloInvalido);
    require!(
        u128_em(16) == vault.indice_ciclo_anterior
            && u128_em(32) == vault.indice_ciclo
            && u128_em(48) == vault.indice_diluicao
            && u64_em(64) == vault.nav_fechamento,
        DomError::RetratoNaoEOMaisRecente
    );
    Ok(d[92] == 0)
}

/// O mínimo para `piso_livre` ler 0..94: discriminador, `ciclo`, os três índices,
/// `nav_fechamento` (64..72), `aberto` (92) e `bump` (93) — 72..92 só no tamanho.
const RETRATO_LIDO: usize = 94;

pub fn handle_publish_nav(
    ctx: Context<PublishNav>,
    nav: u64,
    attestation_hash: [u8; 32],
    timestamp: i64,
) -> Result<()> {
    require!(nav > 0, DomError::InvalidNav);

    let quem = ctx.accounts.publisher.key();
    let pelo_oraculo = quem == ctx.accounts.vault.nav_oracle;
    let pela_mesa = quem == ctx.accounts.vault.authority;
    require!(pelo_oraculo || pela_mesa, DomError::NotOracle); // T35
    let piso_livre = piso_livre(&ctx.accounts.retrato.to_account_info(), &ctx.accounts.vault)?;
    let vault = &mut ctx.accounts.vault;

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

    // -----------------------------------------------------------------------
    // Upgrade J (D-F2-43 §2 e §3): a gaveta e' absorvida no indice e o piso e'
    // recalculado ANTES de aceitar o NAV. O piso = (capital no campo + caixa +
    // P na gaveta) ÷ supply so' sobe dentro do ciclo; NAV bruto abaixo dele e'
    // recusado — foi o que faltou em 20/09, quando o P saiu do preco por 20 h.
    // -----------------------------------------------------------------------
    let supply = ctx.accounts.dom_mint.supply;
    let gaveta_saldo = ctx.accounts.gaveta.amount;
    let gaveta_key = ctx.accounts.gaveta.key();
    let delta_do_lucro =
        crate::indice::absorver_no_cofre(vault, &gaveta_key, gaveta_saldo, supply)?;

    // -----------------------------------------------------------------------
    // ⚠️ UPGRADE N · O PISO E UMA MARCA, NAO UMA FORMULA SOBRE SALDO
    //
    // A regra do dono: toda vez que LUCRO REALIZADO entra na gaveta, o piso
    // passa a ser o NAV BRUTO daquele instante. Exemplo dele: 200k em 200k
    // cotas e' NAV 1,00; realiza 10k -> patrimonio 210k -> NAV 1,05 -> PISO
    // 1,05. Quem entrar depois paga 1,05, porque o lucro ja' esta' no preco.
    //
    // ─── por que a formula antiga tinha de sair ──────────────────────────
    //
    // Ela era `(deployed_usdc + caixa + gaveta_saldo) / supply` — VALOR DE
    // LIVRO. Nasceu no Upgrade J para o laudo mostrar o "NAV liquido" ao lado
    // do bruto, e o comentario dela dizia, em letra grande, que NAO havia
    // recusa por piso. No Upgrade K (D-F2-45) o `deposit.rs` passou a cobrar
    // `max(nav, nav_piso)` e o valor de livro virou PRECO DE ENTRADA sem que
    // ninguem notasse — porque, com o fundo no lucro, os dois numeros ficavam
    // acima do NAV e a diferenca nao aparecia.
    //
    // ⚠️ Com perda nao realizada ela apareceu inteira. MEDIDO em 08/10:
    //
    //     nav ................... 0,989765
    //     piso (formula velha) .. 1,065958     7,70% ACIMA
    //     deployed_usdc ......... 99,3% da base
    //
    // O fundo ficou FECHADO para dinheiro novo: nenhum entrante paga 7,7% de
    // premio. E `deployed_usdc` e' o CUSTO do capital em campo — ele nao cai
    // quando o ativo cai, so' por `return_capital`, `resgate_capital` ou pela
    // caneta do `ajustar_deployed_usdc`.
    //
    // ─── e por que a marca NAO dobra a gaveta ────────────────────────────
    //
    // ⚠️ O NAV publicado JA CONTEM a gaveta: ela e' a perna `mesa` do laudo
    // (`backend-nav/config/pernas.producao.json`, perna[1], rotulo "Lucro do
    // Ciclo"), e o patrimonio e' a soma de TODAS as pernas
    // (`backend-nav/lib/apurador.ts:120-121,153`).
    //
    // Entao `nav_piso = nav + gaveta/supply` DOBRARIA o lucro. Medido em
    // 07/10 01:35, com 4.246,497908 na gaveta: o nav publicado era 1,033575 e
    // `nav + gaveta/supply` daria 1,059333 — 4.246,50 USD contados duas vezes.
    //
    // A marca nao soma nada. Ela GRAVA o NAV, que ja' e' o numero certo.
    //
    // ─── a ordem das linhas e' a regra, nao estilo ───────────────────────
    //
    // A marca grava o NAV **NOVO** (o que esta' sendo publicado), nao o
    // anterior — e' o que o exemplo do dono diz: "realiza 10k -> piso 1,05",
    // e 1,05 e' o NAV DEPOIS do lucro entrar. Por isso o `vault.nav = nav`
    // vem ANTES, e a marca le' `vault.nav`.
    //
    // ⚠️ E ela so' grava quando `delta_do_lucro > 0`. Sem essa condicao, cada
    // publicacao horaria regravaria a marca com o NAV da hora e o piso
    // deixaria de ser marca — viraria "o ultimo NAV", que nao protege ninguem.
    // -----------------------------------------------------------------------
    vault.nav = nav;

    // (3a) Com o retrato aberto o NAV publica e o índice absorve; só a marca espera.
    if delta_do_lucro > 0 && piso_livre {
        vault.nav_piso = nav;
        emit!(PisoMarcado {
            nav_piso: nav,
            delta_do_lucro,
            gaveta_saldo,
            timestamp,
        });
    }

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
