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
    /// O NAV **publicado** (bruto), como sempre — é o preço do fundo.
    pub nav: u64,
    /// Upgrade K (D-F2-45): o que de fato precificou a emissão — `max(nav, nav_piso)`.
    pub preco_usado: u64,
    /// `true` quando o piso venceu o bruto: a cota saiu mais cara que o mercado
    /// das posições, e a tela do aporte tem de dizer isso.
    pub pelo_piso: bool,
}

/// Upgrade K: a correção privilegiada do custo de capital em campo. O motivo
/// viaja no evento porque o memo do Squads não é cadeia.
#[event]
pub struct DeployedAjustado {
    pub antes: u64,
    pub depois: u64,
    pub motivo: String,
    pub timestamp: i64,
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
/// A comissão de um afiliado **não alcançou o mínimo** e foi para a reserva dele.
///
/// ⚠️ **Evento NOVO, e não campo novo no `ComissaoDeAfiliado`** — o cabeçalho
/// daquele evento explica por que campo novo nele é caro: quem decodifica por
/// posição lê o campo errado sem errar em voz alta. Evento novo não desloca
/// nada, e quem não o conhece simplesmente não o vê.
#[event]
pub struct ComissaoAcumulada {
    pub indicado: Pubkey,
    pub afiliado: Pubkey,
    pub bps: u16,
    /// O ganho do indicado no ciclo fechado, pelo índice (micro-USDC).
    pub ganho_indicado: u64,
    /// A comissão DESTE fechamento (micro-USDC). Devida, e debitada da mesa.
    pub comissao_usdc: u64,
    /// O total acumulado na reserva DEPOIS deste fechamento (micro-USDC).
    pub acumulado_micro: u64,
    /// O mínimo vigente (micro-USDC), para o leitor não ter de adivinhá-lo.
    pub minimo_micro: u64,
    pub timestamp: i64,
}

/// A reserva de um afiliado alcançou o mínimo e foi paga INTEIRA em cota.
#[event]
pub struct ComissaoDaReservaPaga {
    pub afiliado: Pubkey,
    /// A comissão deste fechamento (micro-USDC).
    pub do_ciclo_micro: u64,
    /// O que estava acumulado na reserva **antes desta LINHA** (micro-USDC).
    ///
    /// ⚠️ "Antes desta linha", e não "antes deste fechamento": a mesma carteira
    /// pode ser afiliada em duas linhas da mesma lista, e a segunda vê o que a
    /// primeira acumulou. Achado da revisão adversária de 13/10 — o rótulo
    /// anterior dizia "fechamento" e era falso nesse caso.
    pub da_reserva_micro: u64,
    /// `do_ciclo_micro + da_reserva_micro` — o que foi precificado em cota.
    pub total_micro: u64,
    pub timestamp: i64,
}

#[event]
pub struct ComissaoDeAfiliado {
    pub indicado: Pubkey,
    pub afiliado: Pubkey,
    pub bps: u16,
    /// O ganho do indicado no ciclo fechado, pelo índice (micro-USDC).
    pub ganho_indicado: u64,
    /// A comissão **precificada em cota** nesta linha, em micro-USDC.
    ///
    /// ⚠️ Era `ganho_indicado × taxa_mesa × bps ÷ 1e8`, e o doc ficou velho duas
    /// vezes: o `taxa_mesa` saiu em 12/10 (N7: a comissão passou a ser sobre o
    /// ganho BRUTO, `ganho × bps ÷ 1e4`), e no item 7 o campo passou a ser o
    /// **devido** — a comissão deste ciclo **mais** a reserva acumulada que foi
    /// paga junto. A divisão entre as duas parcelas sai no `ComissaoDaReservaPaga`.
    ///
    /// ⚠️ **Consequência para quem reconcilia:** `Σ ComissaoDeAfiliado.comissao_usdc`
    /// deixa de ser igual a `LucroDistribuido.comissoes_afiliados` sempre que uma
    /// reserva é paga — aquele é o DEVIDO do ciclo, este inclui o que veio de
    /// ciclos anteriores. A igualdade só vale somando os `ComissaoAcumulada` e
    /// subtraindo os `ComissaoDaReservaPaga.da_reserva_micro`.
    pub comissao_usdc: u64,
    /// Cotas cunhadas ao afiliado (ao `nav_fechamento`, já com o acerto dele compensado).
    pub cotas: u64,
    pub timestamp: i64,
    // ─── OS CAMPOS DO K VÊM DEPOIS DO `timestamp`, E ISSO É DE PROPÓSITO ─────
    // A mesa mandou (24/09, antes do GO). Campo novo no MEIO da struct desloca
    // tudo que vem atrás: quem decodifica por posição — e não pela planta — leria
    // `cotas_comissao` no lugar do `timestamp` e não erraria em voz alta; leria
    // uma data absurda, ou pior, uma plausível. No fim, o decodificador antigo lê
    // os campos que sempre leu, nas mesmas posições, e simplesmente ignora o
    // rabo novo. É a mesma regra dos erros (6087/6088 no fim do enum, nada
    // renumera) aplicada ao layout do evento.
    /// Upgrade K (achado F9 do CC-DASH): a parte que é COMISSÃO —
    /// `⌊comissao_usdc × 1e6 ÷ nav_fechamento⌋`. Até o J, quem quisesse este
    /// número tinha de derivá-lo fora da cadeia.
    pub cotas_comissao: u64,
    /// A parte que é ACERTO do afiliado, netada na mesma cunhagem (uma vez, na
    /// primeira linha dele). `cotas − cotas_comissao`; **negativo** quando o
    /// afiliado devia taxa e a cunhagem veio menor.
    pub cotas_acerto: i64,
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

/// ⚠️ UPGRADE N · A MARCA DO PISO FOI GRAVADA.
///
/// Sai so' quando lucro realizado ENTRA na gaveta (`delta > 0`) — nao a cada
/// publicacao. E' o unico registro de que o preco de entrada subiu, e por quanto:
/// sem ele, a mesa descobriria o piso novo so' quando alguem reclamasse do preco
/// do aporte.
#[event]
pub struct PisoMarcado {
    /// O NAV bruto que virou piso. E' o mesmo `vault.nav` desta publicacao.
    pub nav_piso: u64,
    /// Quanto de lucro entrou na gaveta nesta leitura. `> 0` sempre, por construcao.
    pub delta_do_lucro: u64,
    /// O saldo da gaveta DEPOIS da entrada — o acumulado do ciclo.
    pub gaveta_saldo: u64,
    pub timestamp: i64,
}

/// ⚠️ UPGRADE N · O PISO FOI RESETADO NO FECHAMENTO.
///
/// A gaveta esvazia no `deposit_especial`, e com ela o motivo do piso. O evento
/// registra o valor QUE SAIU e o que entrou, para o laudo poder explicar a queda
/// do preco de entrada sem que ninguem precise deduzir.
#[event]
pub struct PisoResetado {
    /// O piso que estava marcado, do ciclo que acabou.
    pub piso_anterior: u64,
    /// O novo: o NAV pos-fechamento.
    pub piso_novo: u64,
    pub timestamp: i64,
}

/// A Fase A do fechamento em lotes abriu o retrato de um ciclo.
///
/// ⚠️ `aberto: false` aqui **não** é anomalia: é o fechamento pelo caminho antigo
/// (orçamento zero, comissão paga no próprio laço), em que o retrato é só o
/// registro do ciclo e não há Fase B para rodar.
#[event]
pub struct RetratoAberto {
    pub ciclo: u64,
    pub indice_ciclo_anterior: u128,
    pub indice_ciclo: u128,
    pub nav_fechamento: u64,
    pub orcamento_micro: u64,
    pub linhas_votadas: u16,
    pub aberto: bool,
    pub timestamp: i64,
}

/// A Fase B pagou um lote de comissões.
///
/// ⚠️ Ele traz `linhas_pagas` e `linhas_votadas` juntos **de propósito**: é por
/// essa desigualdade que se vê, de fora, que um ciclo ficou com linha sem pagar —
/// e é o que a decisão 7 do dono manda o `encerrar_ciclo` nomear.
#[event]
pub struct LoteDeComissaoPago {
    pub ciclo: u64,
    pub linhas_do_lote: u16,
    pub linhas_pagas: u16,
    pub linhas_votadas: u16,
    pub gasto_do_lote: u64,
    pub gasto_total: u64,
    pub orcamento_micro: u64,
    /// Quanto deste lote virou **cota** (o resto foi para as reservas).
    pub cunhado_micro: u64,
    /// Quanto da sobra a divisão por 3 **pretendia** devolver à mesa, em micro.
    pub sobra_micro: u64,
    /// ⚠️ Quanto foi **de fato cunhado** aos sócios, em cotas.
    ///
    /// Os dois números divergem de propósito: `shares_from_usdc` trunca a favor do
    /// cofre, e o acerto de cada sócio entra netado na cunhagem. Com sobra de 3
    /// micro, `sobra_micro = 3` e `sobra_em_cotas = 0` — e quem reconcilia precisa
    /// ver os dois, não o pretendido no lugar do real.
    pub sobra_em_cotas: u64,
    pub fechou: bool,
    pub timestamp: i64,
}

/// O ciclo foi ENCERRADO pela mesa com linha não paga (DECISOES 13/10 16:55 (8)).
///
/// ⚠️ **`nao_pagas` é o que esta instrução existe para publicar.** A lista de
/// afiliados não é congelada na Fase A (decisão 9) — só a contagem é —, então este
/// evento é o único lugar onde fica registrado QUEM não recebeu. O contrato prova
/// que cada uma dessas carteiras não tem marca de pago no ciclo e que as contas
/// fecham (`nao_pagas + linhas_pagas == linhas_votadas`); ver o cabeçalho de
/// `instructions/encerrar_ciclo.rs` para o que ele NÃO prova.
#[event]
pub struct CicloEncerrado {
    pub ciclo: u64,
    pub linhas_votadas: u16,
    pub linhas_pagas: u16,
    /// As carteiras das linhas que ficaram SEM pagamento.
    pub nao_pagas: Vec<Pubkey>,
    /// O que foi rateado entre os três sócios, em micro-USDC.
    pub sobra_micro: u64,
    /// O que de FATO virou cota (truncado, a favor do cofre) — não é `sobra_micro`.
    pub sobra_em_cotas: u64,
    pub gasto_micro: u64,
    pub orcamento_micro: u64,
    /// O contador de reservas em aberto, que o encerramento NÃO move.
    pub reservas_em_aberto_micro: u64,
    /// Quantas marcas de pago foram FECHADAS neste encerramento. ⚠️ Pode ser
    /// menor que `linhas_pagas`: fechar marca é opcional (ver o cabeçalho de
    /// `encerrar_ciclo.rs` — exigir todas passava das 64 travas com 42+ linhas).
    pub marcas_fechadas: u16,
    pub rent_devolvido: u64,
    /// Para quem foi o rent das marcas fechadas: o destino nomeado pela mesa,
    /// registrado aqui (18/10 20:45 P3; a marca não guarda pagador).
    pub rent_para: Pubkey,
    pub timestamp: i64,
    /// A parte de CADA sócio, em micro-USDC: `⌊sobra ÷ 3⌋`, a mesma para os três
    /// (regra 7 do devnet, DECISOES 18/10 23:50). ⚠️ É o RATEADO, antes do acerto:
    /// o que cada sócio recebe em cota sai do `cunhar_com_acerto` e pode diferir.
    /// O resto da divisão fica no tesouro (DECISOES 18/10 20:45 P1). No FIM do
    /// evento: quem decodifica por posição não vê os campos anteriores andarem.
    pub por_socio_micro: u64,
}

/// A reserva de comissão de um afiliado foi LIQUIDADA em cota (item 7.6).
#[event]
pub struct ReservaLiquidada {
    pub afiliado: Pubkey,
    /// O que estava acumulado, em micro-USDC.
    pub acumulado_micro: u64,
    /// O NAV **vigente** com que foi convertido (13/10 16:55 (9)).
    pub nav: u64,
    /// As cotas que de fato foram cunhadas — truncadas, a favor do cofre.
    pub cotas: u64,
    /// O contador do retrato mais recente, DEPOIS do decremento.
    pub reservas_em_aberto_micro: u64,
    pub ciclo_do_retrato: u64,
    pub rent_devolvido: u64,
    pub rent_para: Pubkey,
    pub timestamp: i64,
}
