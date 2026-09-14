use {
    crate::{constants::*, error::DomError, events::ParametroAjustado, state::Vault},
    anchor_lang::prelude::*,
};

/// **Qual parâmetro a proposta ajusta.** Upgrade E (`D-F2-21`).
///
/// Um enum e uma instrução, em vez de nove instruções irmãs. A razão é que a
/// validação de cada parâmetro **mora ao lado da atribuição** — separadas, nove
/// arquivos repetiriam a mesma forma e a primeira que esquecesse o `require!`
/// não pareceria diferente das outras.
///
/// O custo é que a proposta chega ao Squads como um discriminante, e não como um
/// nome. **Quem monta a proposta é responsável por dizer, em voz alta, qual
/// parâmetro e qual valor** — como o runbook já manda para toda proposta.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Parametro {
    PrazoDoResgate,
    MinResgateUsdc,
    MinResgateCotas,
    MinSaqueLucro,
    NavStaleness,
    IntervaloDePublicacao,
    CadenciaDeDistribuicao,
    NavBoundPct,
    CapPct,
    ReserveBps,
    PerfFeeBps,
}

/// Ajusta um parâmetro de política. **Privilegiada.**
///
/// # Por que estes valores deixaram de ser constantes
///
/// Princípio fechado pela mesa em 2026-09-08 (`D-F2-21`): **mudança de valor não
/// deve exigir upgrade.** Em duas semanas a mesa pediu três valores diferentes
/// para o piso do resgate; o segundo virou upgrade e o terceiro não vai virar.
///
/// # O que os limites protegem
///
/// A mesa passa a votar número, e **número votado sem limite trava o cofre sem
/// ninguém querer**. Cada `require!` abaixo barra um estado que quebra o fundo,
/// não um valor que alguém achou feio:
///
/// - `staleness` curto demais faz o cofre fechar antes de o oráculo republicar;
/// - `intervalo` maior que a validade cria o ciclo em que o NAV **sempre** vence;
/// - `bound` zero congela o NAV, porque nenhuma variação passa;
/// - `cap` zero recusa todo depósito;
/// - reserva acima de 50% imobiliza o fundo dentro do próprio cofre;
/// - taxa de performance acima de 25% por sócio zera a parcela dos cotistas.
///
/// O último é de natureza diferente dos outros quatro. Aqueles protegem o cofre
/// de um número que o trava; este protege **o cotista da própria mesa**. A mesa
/// vota a taxa que ela mesma recebe, e a parcela do cotista é o resto
/// (`P − 3 × parcela`): sem teto, uma proposta 2/3 levaria o resto a zero sem
/// que nenhum cotista participasse da votação. 2.500 bps cada = 75% no total é
/// o que a mesa fechou em 2026-09-08 como o limite que ela aceita não poder
/// ultrapassar.
///
/// **Os limites continuam sendo constantes, e é de propósito.** Se eles também
/// fossem votáveis, a proteção seria removível pelo mesmo voto que ela protege.
#[derive(Accounts)]
pub struct AjustarParametro<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump = vault.bump,
        has_one = authority @ DomError::Unauthorized,
    )]
    pub vault: Account<'info, Vault>,
}

pub fn handle_ajustar_parametro(
    ctx: Context<AjustarParametro>,
    qual: Parametro,
    novo: u64,
) -> Result<()> {
    let vault = &mut ctx.accounts.vault;

    // O anterior é lido ANTES da atribuição: o evento carrega os dois, e sem o
    // anterior quem audita não sabe se a proposta mudou alguma coisa.
    let anterior: u64 = match qual {
        Parametro::PrazoDoResgate => vault.resgate_capital_prazo as u64,
        Parametro::MinResgateUsdc => vault.min_resgate_capital_usdc,
        Parametro::MinResgateCotas => vault.min_resgate_capital_cotas,
        Parametro::MinSaqueLucro => vault.min_saque_lucro_usdc,
        Parametro::NavStaleness => vault.max_nav_staleness as u64,
        Parametro::IntervaloDePublicacao => vault.min_nav_publish_interval as u64,
        Parametro::CadenciaDeDistribuicao => vault.distribuicao_interval as u64,
        Parametro::NavBoundPct => vault.nav_bound_pct as u64,
        Parametro::CapPct => vault.cap_pct as u64,
        Parametro::ReserveBps => vault.reserve_bps as u64,
        Parametro::PerfFeeBps => vault.perf_fee_bps_total as u64,
    };

    match qual {
        Parametro::PrazoDoResgate => {
            let v = i64::try_from(novo).map_err(|_| error!(DomError::ParametroForaDoLimite))?;
            require!(
                (MIN_PRAZO_VOTAVEL..=MAX_PRAZO_VOTAVEL).contains(&v),
                DomError::ParametroForaDoLimite
            );
            vault.resgate_capital_prazo = v;
        }
        // Os dois pisos do resgate são ligados por OU no `solicitar`, e por isso
        // ZERO em um deles não é inofensivo: zera o piso inteiro, porque o OU
        // passa a ser sempre verdadeiro por aquele lado.
        Parametro::MinResgateUsdc => {
            require!(novo > 0, DomError::ParametroForaDoLimite);
            vault.min_resgate_capital_usdc = novo;
        }
        Parametro::MinResgateCotas => {
            require!(novo > 0, DomError::ParametroForaDoLimite);
            vault.min_resgate_capital_cotas = novo;
        }
        Parametro::MinSaqueLucro => {
            require!(novo > 0, DomError::ParametroForaDoLimite);
            vault.min_saque_lucro_usdc = novo;
        }
        Parametro::NavStaleness => {
            let v = i64::try_from(novo).map_err(|_| error!(DomError::ParametroForaDoLimite))?;
            require!(
                (MIN_STALENESS_VOTAVEL..=MAX_STALENESS_VOTAVEL).contains(&v),
                DomError::ParametroForaDoLimite
            );
            // O intervalo tem de caber DENTRO da validade, senão o oráculo não
            // consegue republicar antes de vencer e o cofre fecha em ciclo.
            require!(
                v > vault.min_nav_publish_interval,
                DomError::ParametrosIncoerentes
            );
            vault.max_nav_staleness = v;
        }
        Parametro::IntervaloDePublicacao => {
            let v = i64::try_from(novo).map_err(|_| error!(DomError::ParametroForaDoLimite))?;
            require!(
                (MIN_INTERVALO_VOTAVEL..=MAX_INTERVALO_VOTAVEL).contains(&v),
                DomError::ParametroForaDoLimite
            );
            require!(v < vault.max_nav_staleness, DomError::ParametrosIncoerentes);
            vault.min_nav_publish_interval = v;
        }
        Parametro::CadenciaDeDistribuicao => {
            let v = i64::try_from(novo).map_err(|_| error!(DomError::ParametroForaDoLimite))?;
            require!(
                (MIN_CADENCIA_VOTAVEL..=MAX_CADENCIA_VOTAVEL).contains(&v),
                DomError::ParametroForaDoLimite
            );
            vault.distribuicao_interval = v;
        }
        Parametro::NavBoundPct => {
            let v = u16::try_from(novo).map_err(|_| error!(DomError::ParametroForaDoLimite))?;
            require!(
                (MIN_BOUND_PCT..=MAX_BOUND_PCT).contains(&v),
                DomError::ParametroForaDoLimite
            );
            vault.nav_bound_pct = v;
        }
        Parametro::CapPct => {
            let v = u16::try_from(novo).map_err(|_| error!(DomError::ParametroForaDoLimite))?;
            require!(
                (MIN_CAP_PCT..=MAX_CAP_PCT).contains(&v),
                DomError::ParametroForaDoLimite
            );
            vault.cap_pct = v;
        }
        Parametro::ReserveBps => {
            let v = u16::try_from(novo).map_err(|_| error!(DomError::ParametroForaDoLimite))?;
            require!(
                v <= MAX_RESERVE_BPS_VOTAVEL,
                DomError::ParametroForaDoLimite
            );
            vault.reserve_bps = v;
        }
        Parametro::PerfFeeBps => {
            let v = u16::try_from(novo).map_err(|_| error!(DomError::ParametroForaDoLimite))?;
            // O PISO entra no binario junto com o teto — `D-F2-35`. A mesa
            // publicou "50% do lucro realizado"; sem piso, esse numero era
            // palavra dada. Com ele, e' a parte que a votacao nao alcanca, do
            // mesmo jeito que o teto protege a parcela dos cotistas por cima.
            require!(
                (MIN_PERF_FEE_BPS_TOTAL..=MAX_PERF_FEE_BPS_TOTAL).contains(&v),
                DomError::ParametroForaDoLimite
            );
            vault.perf_fee_bps_total = v;
        }
    }

    emit!(ParametroAjustado {
        qual: qual as u8,
        anterior,
        novo,
    });
    Ok(())
}
