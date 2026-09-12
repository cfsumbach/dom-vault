use anchor_lang::prelude::*;

/// PDA único de estado do cofre.
#[constant]
pub const VAULT_SEED: &[u8] = b"vault";

/// PDA por carteira: `[WHITELIST_SEED, owner]`.
#[constant]
pub const WHITELIST_SEED: &[u8] = b"whitelist";

/// Autoridade das contas do programa (escrow de cotas em `request_redeem`).
/// É PDA, não chave humana — por isso é isenta de whitelist e de cap (D2/T24).
#[constant]
pub const ESCROW_SEED: &[u8] = b"escrow";

/// Seed da conta de validação da transfer-hook-interface:
/// `[EXTRA_ACCOUNT_METAS_SEED, mint]`.
///
/// O literal é o mesmo de `spl_transfer_hook_interface`, onde a constante é
/// privada. `get_extra_account_metas_address` do crate é a referência; se um dia
/// divergir, o `initialize_extra_account_meta_list` falha na conferência de
/// endereço em vez de gravar num PDA errado.
#[constant]
pub const EXTRA_ACCOUNT_METAS_SEED: &[u8] = b"extra-account-metas";

/// Caixa do fundo em USDC. PDA `[TREASURY_SEED]`, autoridade = PDA do cofre.
#[constant]
pub const TREASURY_SEED: &[u8] = b"treasury";

/// Cotas em escrow entre o pedido e o processamento (D8). Conta de token do
/// DOM cuja autoridade é o PDA `[ESCROW_SEED]`.
#[constant]
pub const ESCROW_DOM_SEED: &[u8] = b"escrow-dom";

/// Um pedido de resgate de capital. PDA `[RESGATE_SEED, id_le_bytes]`.
///
/// **Conta por pedido, e não vetor no cofre.** A F2 tinha uma fila de tamanho
/// fixo (32 posições) porque havia um pedido por carteira. Com resgate parcial
/// permitido, N pedidos por carteira, qualquer teto de vetor vira o próximo bug —
/// e vetor dentro do cofre faria o rent de um pedido depender de quantos outros
/// existem. O contador global custa 8 bytes e não tem limite.
#[constant]
pub const RESGATE_SEED: &[u8] = b"resgate";

/// **D+180 — o teto do prazo de resgate de capital.**
///
/// Teto, não carência: a mesa pode efetivar antes se a saúde da posição permitir,
/// e o contrato **não** exige que o prazo tenha vencido para pagar. O que ele faz
/// com esta data é o contrário — passado o vencimento sem pagamento, o pedido
/// pode ser marcado em atraso e o cofre para de mandar capital para campo.
#[constant]
pub const RESGATE_CAPITAL_PRAZO: i64 = 180 * 24 * 60 * 60;

/// Reserva de 10% do NAV (D7). Intocável por `redeem_fee_share`, **consumível**
/// por `process_redemptions` — a reserva existe para a fila.
#[constant]
pub const RESERVE_BPS: u16 = 1_000;

/// Livro de `fee_share` por sócio. PDA `[FEE_SHARE_SEED, socio]`.
///
/// `fee_share` e cota comum são o **mesmo mint**: a origem é flag em estado do
/// programa, não no token account (D3.1). Este livro é essa flag.
#[constant]
pub const FEE_SHARE_SEED: &[u8] = b"fee-share";

/// Taxa de performance **por sócio**, em pontos-base do **lucro realizado do
/// ciclo** (`P`). 2.000 bps = 20% para cada uma das três carteiras — ver D21.
///
/// **A base mudou (D-F2-09).** Era "o que passou do high water mark", régua do
/// fundo tradicional que a mesa não adotou: ela cobrava sobre valorização de
/// NAV, inclusive a que veio só de marcação, e transformava marcação em caixa
/// saindo do cofre pelo `redeem_fee_share`. Agora a base é `P` — dinheiro que
/// **voltou** para o caixa e entrou pelo `deposit_especial`. Não há como
/// inventar `P`: ele é o valor transferido na própria instrução.
///
/// 3 × 20% = 60% para os sócios, 40% para os cotistas via NAV.
#[constant]
pub const PERF_FEE_BPS_POR_SOCIO: u16 = 2_000;

/// Quantos sócios. Não é configurável: o 20/20/20 pressupõe três destinos.
pub const NUM_SOCIOS: usize = 3;

pub const BPS_DEN: u128 = 10_000;

/// Cap de concentração por carteira: 25% do supply (D3).
/// Numerador e denominador separados para manter aritmética inteira (D9).
pub const CAP_NUM: u128 = 25;
pub const CAP_DEN: u128 = 100;

/// NAV com 6 casas: `1_000_000` = 1,000000 (D9). Mesma escala do token e do
/// USDC, então cota e USDC compartilham a unidade e não há conversão de escala.
#[constant]
pub const NAV_SCALE: u64 = 1_000_000;

/// NAV da gênese = 1,000000. Primeiro depósito sai 1:1 (T01).
#[constant]
pub const NAV_GENESIS: u64 = NAV_SCALE;

/// Depósito mínimo: 200,000000 USDC. **Limite inclusivo** — 200 exatos entram,
/// 199,999999 não (T03/T04).
#[constant]
pub const MIN_DEPOSIT: u64 = 100 * NAV_SCALE;

// =============================================================================
// Bloco de estado da F2 — os achados da auditoria, num `space` só.
// =============================================================================

/// DV11 — idade máxima do NAV para **leitura**. 26h, não 24h: a cadência-alvo
/// de publicação é diária, e um teto justo brica o cofre quando o cron atrasa
/// duas horas. A regra é "2× a cadência", e o número muda junto com ela.
///
/// **Staleness sozinho não para o vetor do DV11**: oráculo comprometido publica
/// NAV fresco e falso. Quem para é o `NAV_BOUND`. Este serve para o backend
/// caído — e é constante, não campo: teto configurável seria mais uma superfície
/// privilegiada para o mesmo efeito.
pub const MAX_NAV_STALENESS: i64 = 26 * 60 * 60;

/// DV11 — variação máxima do NAV entre publicações **do oráculo**: 15%.
///
/// É este que para o oráculo comprometido: `NAV = 0,000001` não passa. A mesa
/// atravessa o limite por proposta (ver `publish_nav`), então um dia de queda
/// real não trava o cofre — a válvula é o que torna 15% seguro em vez de
/// apertado.
pub const NAV_BOUND_NUM: u128 = 15;
pub const NAV_BOUND_DEN: u128 = 100;

/// Resgate de capital mínimo, em **cotas**. 5.000 DOM.
///
/// **Piso de negócio** (mesa, 2026-08-25): não se abre um processo de seis meses,
/// com voto do multisig, para migalha.
///
/// # Em USDC, e não em cotas — a razão MUDOU no Upgrade D
///
/// Este piso nasceu em **cotas**, e a justificativa era a `D-F2-03`:
///
/// > Mínimo por valor exigiria ler o NAV dentro do pedido, e aí a recusa passaria
/// > a depender da frescura do oráculo.
///
/// **A justificativa não se sustentava, e o custo dela era alto.**
///
/// Não se sustentava porque o `solicitar_resgate_capital` **já lê `vault.nav`** —
/// é dele que sai o `nav_na_solicitacao`, o teto do pior-NAV. Comparar contra o
/// mesmo número não acrescenta acoplamento nenhum ao oráculo.
///
/// E o que a `D-F2-03` protege continua protegido: **no `solicitar` não se move
/// dinheiro.** A cota vai para o escrow e o preço se decide no `efetivar`, com
/// `require_nav_fresco` e três travas por cima. NAV velho no portão de admissão
/// não paga errado — no máximo admite ou barra um pedido na borda.
///
/// O custo era este: com piso em cotas, **quem tem 999 cotas precisa COMPRAR MAIS
/// para poder SAIR.** Obrigar alguém a investir mais para ter o direito de
/// desinvestir não é piso de negócio, é armadilha.
///
/// ## São DOIS pisos, ligados por OU — e o segundo não é enfeite
///
/// ```text
/// cotas >= MIN_RESGATE_CAPITAL_COTAS  ||  cotas × nav >= MIN_RESGATE_CAPITAL_USDC
/// ```
///
/// O piso em USDC sozinho **fecha a saída quando o NAV cai** — e é quando o
/// investidor mais quer sair. Não é hipótese: os tripwires `T75` e `T90` pegaram
/// isso na hora. Com queda de 20% (NAV 0,80), 1.000 cotas valem 800 USDC e o
/// pedido era **recusado**; ao NAV de pó, ninguém no mundo teria cota bastante.
///
/// Com o **OU**, cada piso cobre o furo do outro:
///
/// | quem | passa por |
/// |---|---|
/// | comprou pouco, a cota valorizou | **valor** — 500 cotas a NAV 3,0 são 1.500 USDC |
/// | tem 1.000 cotas, o NAV desabou | **cotas** — imune a preço, a porta não fecha |
/// | poeira | nenhum dos dois |
///
/// **O piso em cotas é a trava anti-colapso; o piso em USDC é a porta do pequeno.**
/// Remover qualquer um dos dois reabre um furo que já foi visto.
///
/// ## E mede o que a régua tem de medir
///
/// Mesma razão do `MIN_SAQUE_LUCRO_USDC`: piso em cotas mede *quantas unidades
/// você comprou*; a régua precisa medir *quanto você está tirando*. **Piso de
/// negócio mede negócio, e negócio é dinheiro.**
///
/// ## Não confundir com o piso do saque de lucro — são operações diferentes
///
/// ```text
/// MIN_SAQUE_LUCRO_USDC       valor = só o LUCRO do ciclo   (cotas × delta)
/// MIN_RESGATE_CAPITAL_USDC   valor = CAPITAL + valorização (cotas × nav)
/// ```
///
/// As duas réguas são em USDC e medem coisas distintas. **Isso é coerência, não
/// inconsistência** — quem "uniformizar" as duas quebra uma das duas.
///
/// Valor: a mesa fechou 5.000, baixou para 1.000 no Upgrade D (o critério de
/// saída do piloto exigia um resgate provado em mainnet, e 5.000 obrigava a mover
/// 5.000 USDC reais num piloto de valores simbólicos). Ao NAV de gênese, 1.000
/// cotas eram 1.000 USDC — a troca de unidade mantém a paridade de partida.
pub const MIN_RESGATE_CAPITAL_USDC: u64 = 100 * NAV_SCALE;

/// Trava anti-colapso do resgate: **em COTAS, e imune ao preço**. Ver o OU
/// documentado acima — sozinho ele excluía quem comprou pouco, e é por isso que
/// não é mais o único.
pub const MIN_RESGATE_CAPITAL_COTAS: u64 = 100 * NAV_SCALE;

/// **Piso do saque de lucro, em USDC — não em cotas.** *(Upgrade D, 2026-08-31)*
///
/// A razão da unidade é do modelo, e é o que impede alguém de "corrigir" isto
/// para cotas mais tarde: **no modelo de NAV a cota não se multiplica com o
/// lucro.** O cotista termina o ciclo com a mesma quantidade de cotas e cada uma
/// vale mais. Piso em cotas mediria o tamanho da posição; o que a régua precisa
/// medir é **quanto dinheiro o saque paga** — e dinheiro é USDC.
///
/// A comparação é sobre o `valor` que o `sacar_lucro` calcula, que é o **lucro**,
/// não o principal: `cotas × delta_lucro_por_cota`, limitado pelo bolo. Quem tem
/// 199 USDC de lucro no ciclo **não saca naquele ciclo** — e não perde nada, o
/// valor fica no preço da cota e volta na próxima distribuição.
///
/// Na escala do USDC, que é a mesma do NAV (6 casas).
pub const MIN_SAQUE_LUCRO_USDC: u64 = 100 * NAV_SCALE;

// -----------------------------------------------------------------------------
// **Não há teto de pedidos por carteira no resgate de capital.**
//
// A F2 tinha `MAX_PEDIDOS_POR_CARTEIRA = 1` (D-F2-13) para proteger uma fila de
// 32 posições: com um pedido por carteira, encher a fila exigia 32 carteiras
// aprovadas pela mesa, e aprovação é coisa que a mesa controla uma a uma.
//
// O resgate de capital **não tem fila**. Cada pedido é uma conta própria, paga
// pelo próprio rent de quem pede, e resgate parcial é permitido por decisão da
// mesa — cada fração com sua data e seu pior-NAV. Não existe recurso comum a
// esgotar, então não existe o ataque que aquele teto barrava.
//
// O que impede pedido de poeira aqui sao os dois pisos (COTAS ou USDC), e o que impede
// pedir duas vezes sobre a mesma cota é o escrow: cota travada saiu da carteira.
// -----------------------------------------------------------------------------

/// DV10 — intervalo mínimo entre distribuições de lucro. **Quinzena.**
///
/// **O motivo da trava mudou junto com a régua (D-F2-09).** Ela existia contra
/// cobrar taxa sobre repique intradia de NAV: apurando todo dia, cada subida
/// virava taxa e as quedas entre elas não devolviam nada. Com a base virando
/// `P` — dinheiro que entrou em caixa — esse abuso deixou de existir: não se
/// inventa `P`.
///
/// O que ela impede agora é **ritmo**: pulverizar uma distribuição em trinta
/// depósitos por dia picotaria a emissão de cota de sócio e abriria e fecharia
/// a janela de saque de lucro sem que cotista nenhum conseguisse usá-la.
///
/// Era trimestre porque a apuração era trimestral. O ciclo operacional da mesa
/// é quinzenal, e trimestre compilado bloquearia o ciclo real.
///
/// O `CYCLE_LEN` de 30 dias **saiu** junto com a fila D+30: ele era a janela do
/// cap de resgate por ciclo, e não há mais ciclo de resgate. A distribuição
/// quinzenal e o prazo de resgate de capital (`RESGATE_CAPITAL_PRAZO`, 180 dias)
/// são calendários independentes — cerca de doze distribuições dentro do prazo
/// máximo de um resgate.
pub const DISTRIBUICAO_INTERVAL: i64 = 15 * 24 * 60 * 60;

/// Vagas de destino operacional de `deploy_capital`. **Duas, e não crescem.**
///
/// `SAFE-BASE` é endereço EVM: não tem ATA, não tem owner SPL, não pode ser
/// destino de `transfer_checked`. A perna Base se alcança por CCTP a partir do
/// `VAULT-SQUADS`, e quem prende o endereço do lado de lá é o Safe/Zodiac.
///
/// D-F2-09: a mesa fechou em **uma carteira operacional só** recebendo saque do
/// cofre — a divisão entre as pernas (Aave/Base, Jupiter/Solana, ponte Exness)
/// acontece **dela para frente**, fora do cofre. Uma vaga ocupada, uma livre.
///
/// O array fica onde está de propósito. Ele mora no byte 448 (era 444 antes do
/// resgate de capital), **antes** dos bumps e do `min_deposit`: crescê-lo
/// deslocaria 11 bytes de conta viva e
/// exigiria uma migração com memmove — o tipo de operação que, errada, zera os
/// bumps e mata o PDA com dinheiro dentro. Precisando de mais destino um dia, a
/// saída é conta por destino (padrão do `WhitelistEntry`), nunca esticar isto.
pub const DEPLOY_ALLOWLIST_LEN: usize = 2;

/// Marca de saque de lucro por carteira: PDA `[LUCRO_SEED, cotista]`.
///
/// Guarda **um** `i64`: o `ultima_distribuicao_ts` do último saque daquela
/// carteira. Sem ela o mesmo cotista saca em laço dentro da mesma janela — o
/// direito é `cotas × delta`, e `cotas` continua positivo depois do pagamento.
///
/// Criada preguiçosamente, no primeiro saque. Não toca o `deposit`, não entra
/// na gênese, não custa nada a quem nunca sacar.
#[constant]
pub const LUCRO_SEED: &[u8] = b"lucro";

/// **Versão do layout da conta `Vault`. Sobe a cada upgrade que mude o layout.**
///
/// `1` foi o layout que nasceu com o campo — o do resgate de capital, 541 bytes.
/// Layouts anteriores (519 e 527) não têm o campo e são reconhecidos pelo
/// tamanho, que naqueles casos ainda distinguia.
///
/// O `Vault.layout_version` é conferido pelas instruções contra esta constante.
/// O que ela fecha está provado em
/// `tests/estado_f2.rs::layout_do_mesmo_tamanho_desserializa_e_so_uma_trava_acidental_barra`:
/// dois layouts do mesmo tamanho passam pelo discriminador e pela guarda de
/// tamanho, e o que barra hoje são travas acidentais dependentes de dado.
#[constant]
pub const LAYOUT_VERSION: u16 = 2;

// -----------------------------------------------------------------------------
// LIMITES DOS PARÂMETROS DE POLÍTICA — Upgrade E
// -----------------------------------------------------------------------------
// A mesa passa a votar valor. Estes limites dizem o que ela NAO pode votar — e
// existem porque parametro sem limite e' um caminho para o cofre travar por voto
// distraido, sem ninguem querer.
//
// Cada um barra um estado que quebra o fundo, nao um valor "feio":
// staleness zero congela tudo na hora seguinte; bound zero congela o NAV porque
// nenhuma variacao passa; cap zero impede qualquer deposito.
// -----------------------------------------------------------------------------

/// O NAV precisa valer por pelo menos uma hora, ou o cofre trava sozinho.
pub const MIN_STALENESS_VOTAVEL: i64 = 60 * 60;
/// E no maximo uma semana: acima disso "fresco" deixa de significar algo.
pub const MAX_STALENESS_VOTAVEL: i64 = 7 * 24 * 60 * 60;
/// Publicar sem intervalo nenhum tira a defesa de taxa da D-F2-06.
pub const MIN_INTERVALO_VOTAVEL: i64 = 60;
/// Intervalo maior que a propria validade do NAV faria o oraculo nao conseguir
/// republicar antes de vencer — o cofre fecharia sozinho, em ciclo.
pub const MAX_INTERVALO_VOTAVEL: i64 = 24 * 60 * 60;
/// Prazo de resgate: de uma semana a cinco anos.
pub const MIN_PRAZO_VOTAVEL: i64 = 7 * 24 * 60 * 60;
pub const MAX_PRAZO_VOTAVEL: i64 = 5 * 365 * 24 * 60 * 60;
/// Cadencia de distribuicao: de um dia a um ano.
pub const MIN_CADENCIA_VOTAVEL: i64 = 24 * 60 * 60;
pub const MAX_CADENCIA_VOTAVEL: i64 = 365 * 24 * 60 * 60;
/// O limite de variacao do NAV nao pode ser 0 (congela) nem 100 (some).
pub const MIN_BOUND_PCT: u16 = 1;
pub const MAX_BOUND_PCT: u16 = 50;
/// O teto de concentracao nao pode ser 0 (nenhum deposito passa).
pub const MIN_CAP_PCT: u16 = 1;
pub const MAX_CAP_PCT: u16 = 100;
/// A reserva nao pode ser o patrimonio inteiro.
pub const MAX_RESERVE_BPS_VOTAVEL: u16 = 5_000;

/// **Teto da taxa de performance por sócio, em pontos-base. 2.500 = 25%.**
///
/// A mesa fechou o teto em 2026-09-08: **75% no total**, que com três sócios dá
/// 25% cada. O valor em vigor continua sendo 2.000 (20% cada, 60% total) — o
/// teto não é a taxa, é o quanto uma votação futura pode chegar a subir.
///
/// Existir teto é o ponto. Sem ele, o campo votável seria uma porta para a
/// mesa se pagar 100% do lucro realizado por proposta, e a parcela dos
/// cotistas (`P − 3 × parcela`) iria a zero sem que nenhum cotista votasse.
/// O teto é a promessa que o binário sustenta e a votação não alcança.
pub const MAX_PERF_FEE_BPS_POR_SOCIO: u16 = 2_500;

/// **Intervalo mínimo entre publicações do ORÁCULO. Uma hora.**
///
/// O limite de 15% por publicação (`NAV_BOUND`) protege contra o dedo gordo de um
/// operador honesto. Ele **não** protegia contra quem tem a chave: não havia
/// intervalo mínimo, a monotonicidade só exige `timestamp > nav_ts`, e o relógio
/// da rede tem granularidade de um segundo. Teto: uma publicação por segundo.
///
/// ```text
/// 0,85^5 = 0,4437   cinco segundos levavam o NAV a menos da metade
/// 1,15^5 = 2,0114   cinco segundos dobravam o NAV
/// ```
///
/// **O limite de 15% comprava cinco segundos.** Com intervalo de uma hora ele
/// vira limite de TAXA: dobrar o NAV passa a levar cinco horas, que é janela em
/// que gente humana percebe e reage — e agora tem com o que reagir, porque o
/// `set_nav_oracle` existe.
///
/// **A válvula da mesa não tem intervalo.** Quem atravessa por proposta, com dois
/// votos, é caminho de crise: pôr carência nele seria travar o cofre justamente
/// no dia em que ele precisa ser destravado.
#[constant]
pub const MIN_NAV_PUBLISH_INTERVAL: i64 = 60 * 60;
