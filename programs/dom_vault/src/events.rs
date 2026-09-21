use anchor_lang::prelude::*;

#[event]
pub struct VaultInitialized {
    pub authority: Pubkey,
    pub nav_oracle: Pubkey,
    pub socios: [Pubkey; 3],
    pub dom_mint: Pubkey,
    pub usdc_mint: Pubkey,
    pub treasury: Pubkey,
    pub escrow_authority: Pubkey,
    pub nav: u64,
}

/// `nav` é o do instante do depósito — o mesmo que gerou as cotas. Fica no
/// evento para que a conciliação não dependa de reler o estado depois.
#[event]
pub struct Deposited {
    pub depositor: Pubkey,
    pub usdc: u64,
    pub shares: u64,
    pub nav: u64,
}

/// T55 — toda atualização de whitelist emite evento.
#[event]
pub struct WhitelistUpdated {
    pub owner: Pubkey,
    pub active: bool,
    /// `0` = sem excecao; vale o piso do cofre. Ver `D-F2-30`.
    pub min_deposit_proprio: u64,
}

/// T11 — evento na ativação do cap. Só existe uma vez: a ativação é one-way (D3).
#[event]
pub struct CapEnforcementEnabled {
    pub authority: Pubkey,
}

/// T37 — NAV, hash de atestação e timestamp saem juntos. O hash é o que liga o
/// número publicado à apuração que o produziu; NAV sem atestação é opinião.
#[event]
pub struct NavPublished {
    pub nav: u64,
    pub attestation_hash: [u8; 32],
    pub timestamp: i64,
    pub publisher: Pubkey,
    /// NAV anterior, para o leitor do evento medir a variação sem precisar do
    /// evento anterior — que pode já ter expirado da retenção do RPC.
    pub nav_anterior: u64,
    /// `true` quando a publicação passou **pela válvula da mesa**, fora do
    /// limite de variação do oráculo. É fato de transparência, não detalhe de
    /// implementação: NAV que atravessou o limite tem que ser visível.
    pub pela_valvula: bool,
}

/// Um pedido de resgate de capital aberto.
///
/// O `vence_ts` vai no evento porque é a data que o dashboard mostra e a que o
/// `marcar_vencido` cobra — e porque quem indexa não deveria precisar recalcular
/// `solicitado_ts + prazo` com a constante certa do momento certo.
#[event]
pub struct ResgateSolicitado {
    pub id: u64,
    pub owner: Pubkey,
    pub cotas: u64,
    pub nav_na_solicitacao: u64,
    pub solicitado_ts: i64,
    pub vence_ts: i64,
}

/// Um resgate de capital pago.
///
/// **Este evento é a peça de auditoria do pior-NAV.** O contrato não consegue
/// provar que o `pior_nav` é o mínimo do período (§ do `resgate_capital.rs`);
/// quem prova é quem enumera os `NavPublished` entre `solicitado_ts` e
/// `efetivado_ts` e refaz o mínimo. Por isso os dois carimbos vão aqui, junto do
/// número usado — sem eles a conferência exigiria adivinhar o intervalo.
#[event]
pub struct ResgateEfetivado {
    pub id: u64,
    pub owner: Pubkey,
    pub cotas: u64,
    pub pior_nav: u64,
    pub valor: u64,
    pub solicitado_ts: i64,
    pub efetivado_ts: i64,
    pub nav_attestation: [u8; 32],
}

/// Um pedido passou do prazo sem pagamento.
///
/// Carrega o `total_em_atraso` porque é ele que trava o `deploy_capital`: quem
/// lê o evento consegue dizer se o cofre está impedido de mandar capital para
/// campo sem precisar ler a conta.
#[event]
pub struct ResgateEmAtraso {
    pub id: u64,
    pub owner: Pubkey,
    pub cotas: u64,
    pub vence_ts: i64,
    pub marcado_ts: i64,
    pub total_em_atraso: u32,
}

/// D-F2-09 — uma distribuição de lucro realizado.
///
/// Carrega o par de NAVs porque a emissão de cota de sócio dilui: sem
/// antes/depois não dá para conferir a distribuição sem refazer a conta. E
/// carrega o `P` de entrada porque **ele é a base da taxa** — quem audita
/// precisa ver que 60/40 saiu de um número que entrou em caixa, não de uma
/// valorização de marcação.
#[event]
pub struct LucroDistribuido {
    /// `P` — o lucro realizado que entrou na treasury nesta instrução.
    pub lucro_realizado: u64,
    /// 40% de `P`. Sobe o NAV, e é o teto do bolo de saque.
    pub parcela_cotistas: u64,
    /// 60% de `P`. Vira cota nova nas três carteiras de sócio.
    pub parcela_socios: u64,
    pub nav_antes: u64,
    pub nav_depois: u64,
    /// `nav_depois - nav_antes`. É o que cada cota rende nesta distribuição.
    pub delta_lucro_por_cota: u64,
    pub cotas_por_socio: u64,
    /// J1: Σ comissões de afiliados, em USDC — saiu de dentro de `parcela_socios`.
    pub comissoes_afiliados: u64,
    pub timestamp: i64,
}

/// J1: uma comissão de afiliado cunhada no fechamento. Um evento por linha da
/// lista — o arquivo de distribuição é gerado daqui.
#[event]
pub struct ComissaoDeAfiliado {
    pub indicado: Pubkey,
    pub afiliado: Pubkey,
    pub bps: u16,
    /// O ganho do indicado no ciclo fechado, pelo índice (micro-USDC).
    pub ganho_indicado: u64,
    /// `ganho_indicado × taxa_mesa × bps ÷ 1e8` (micro-USDC).
    pub comissao_usdc: u64,
    /// Cotas cunhadas ao afiliado (ao `nav_fechamento`, já com o acerto dele compensado).
    pub cotas: u64,
    pub timestamp: i64,
}

/// D-F2-09 — um cotista sacou o lucro da janela.
///
/// `restante` sai no evento para o backend acompanhar o esvaziamento do bolo
/// sem precisar reler a conta a cada saque.
#[event]
pub struct LucroSacadoPorCotista {
    pub cotista: Pubkey,
    /// Cotas queimadas para pagar. O saque devolve o cotista exatamente à
    /// posição em USDC que ele tinha **antes** da distribuição.
    pub cotas_queimadas: u64,
    pub usdc: u64,
    pub nav: u64,
    pub restante: u64,
}

/// D-F2-09 — a janela de saque fechou porque o capital voltou a campo.
#[event]
pub struct JanelaDeLucroEncerrada {
    /// O que sobrou no bolo sem ser sacado. Não some do fundo: continua no
    /// preço da cota de quem não sacou.
    pub nao_sacado: u64,
    pub timestamp: i64,
}

#[event]
pub struct FeeShareRedeemed {
    pub socio: Pubkey,
    pub shares: u64,
    pub usdc: u64,
    pub nav: u64,
}

#[event]
pub struct SociosUpdated {
    pub socios: [Pubkey; 3],
}

#[event]
pub struct PauseToggled {
    pub paused: bool,
    pub authority: Pubkey,
}

/// T33 — virada de ciclo, detectada preguiçosamente na primeira chamada do
/// ciclo novo. Sem cron: quem vira o ciclo é quem chega primeiro depois dele.
#[event]
pub struct CycleRolled {
    pub cycle_index: u64,
    pub previous_redeemed: u64,
}

// ---------------------------------------------------------------------------
// Bloco 3 da F2 — a porta operacional
// ---------------------------------------------------------------------------

/// Capital saindo da treasury para um destino operacional allowlisted.
///
/// `treasury_pos` e `deployed_usdc` viajam no evento para o backend de NAV
/// fechar a conta sem precisar de outra chamada de RPC — mas a fonte de verdade
/// é o **estado**, não este evento: evento expira da janela de retenção do RPC
/// (a lição da DV12).
#[event]
pub struct CapitalDeployed {
    pub valor: u64,
    pub destino: Pubkey,
    pub treasury_pos: u64,
    pub deployed_usdc: u64,
    pub timestamp: i64,
}

/// Capital voltando para a treasury. Entrar é irrestrito: qualquer um pode
/// devolver ao cofre.
#[event]
pub struct CapitalReturned {
    pub valor: u64,
    pub origem: Pubkey,
    pub treasury_pos: u64,
    pub deployed_usdc: u64,
    pub timestamp: i64,
}

#[event]
pub struct DeployAllowlistUpdated {
    pub indice: u8,
    pub destino: Pubkey,
    pub anterior: Pubkey,
}

/// Mudança do piso de aporte. `anterior: 0` marca a migração — antes dela o
/// campo não existia.
#[event]
pub struct MinDepositUpdated {
    pub anterior: u64,
    pub novo: u64,
}

/// A mesa nomeou (ou trocou) a conta de onde os resgates de capital são pagos.
#[event]
pub struct EnderecoDeResgateAtualizado {
    pub anterior: Pubkey,
    pub novo: Pubkey,
}

/// A mesa rotacionou a chave do oráculo de NAV.
///
/// Vai a evento porque rotação é fato de segurança: quem indexa precisa saber a
/// partir de qual slot uma publicação assinada pela chave velha deixa de ser
/// legítima.
#[event]
pub struct NavOracleTrocado {
    pub anterior: Pubkey,
    pub novo: Pubkey,
}

/// Um parâmetro de política ajustado por proposta — Upgrade E.
///
/// Carrega o anterior **e** o novo porque quem audita precisa saber se a
/// proposta mudou alguma coisa. Proposta que grava o mesmo valor passa, gasta
/// dois votos e não aparece em lugar nenhum sem este par.
#[event]
pub struct ParametroAjustado {
    /// O discriminante do `Parametro`. `u8` para o evento não depender do enum.
    pub qual: u8,
    pub anterior: u64,
    pub novo: u64,
}
