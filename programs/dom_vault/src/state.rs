use {
    crate::constants::{DEPLOY_ALLOWLIST_LEN, NUM_SOCIOS},
    anchor_lang::prelude::*,
};

/// Estado global do cofre. PDA `[VAULT_SEED]`.
///
/// Cresce por bloco, com o campo entrando junto da instrução que o escreve
/// (P3).
///
/// **Layout, 527 bytes** (8 de discriminador + 519 de dados). O `hwm` saiu no
/// D-F2-09 e todos os campos entre o byte 384 e o 519 andaram 8 bytes para
/// trás; `delta_lucro_por_cota` e `lucro_sacavel_restante` entraram no fim.
/// A conta viva de devnet foi de 519 para 527 por `migrate_vault_lucro`, que
/// desloca a cauda **antes** de zerar o rabo — nessa ordem, senão os bumps
/// morrem e o PDA para de assinar com dinheiro dentro. Mainnet nasce no
/// formato novo e não conhece essa instrução.
#[account]
#[derive(InitSpace)]
pub struct Vault {
    /// Vault PDA do Squads v4 (D2). Assina toda instrução privilegiada.
    pub authority: Pubkey,
    /// As três carteiras que recebem `fee_share`. **Distintas par a par**, em
    /// todo ponto de escrita, não só no `initialize` (D10).
    pub socios: [Pubkey; NUM_SOCIOS],
    /// Quem publica NAV. Separado da autoridade de propósito: publicar NAV é
    /// operação de rotina do backend, não ato de governança (T35).
    pub nav_oracle: Pubkey,
    /// Allowlist de um item só: o mint do DOM (D5.1).
    pub dom_mint: Pubkey,
    /// Mint do USDC aceito em `deposit`. Também é allowlist de um item: sem
    /// isso, um depósito em token de brinquedo compraria cotas de verdade.
    pub usdc_mint: Pubkey,
    /// Caixa do fundo em USDC. PDA `[TREASURY_SEED]`.
    pub treasury: Pubkey,
    /// Cotas em escrow entre pedido e processamento. PDA `[ESCROW_DOM_SEED]`.
    pub escrow_dom: Pubkey,
    /// PDA `[ESCROW_SEED]`. Autoridade das contas do programa — isenta de
    /// whitelist e de cap (D2/T24/D12).
    pub escrow_authority: Pubkey,
    /// NAV com 6 casas. Nasce em `NAV_GENESIS` e só muda por `publish_nav`.
    /// **`deposit` não mexe** — novato não dilui antigo (T02).
    pub nav: u64,
    /// Timestamp do último NAV publicado. Monotônico (T36).
    pub nav_ts: i64,
    /// Soma das cotas travadas em pedidos de resgate de capital abertos.
    ///
    /// Invariante que o fuzz confere em toda parada: o saldo de `escrow_dom`
    /// nunca é menor que isto.
    ///
    /// **Renomeado de `escrowed_shares`** quando a fila D+30 saiu. O sentido é o
    /// mesmo — cota que saiu da carteira e está sob guarda do programa esperando
    /// pagamento —, mudou o mecanismo que a coloca lá. Não é reaproveitamento de
    /// byte morto: é o mesmo dado com o nome certo, como o
    /// `last_accrue_ts → ultima_distribuicao_ts` do D-F2-09.
    pub cotas_travadas_resgate: u64,
    /// Reserva em pontos-base do NAV (D7). 1000 = 10%.
    pub reserve_bps: u16,
    /// `pause` bloqueia entrada e pedido novo; **não** bloqueia o processamento
    /// da fila que já existe (M9/T48).
    pub paused: bool,
    /// D3: nasce `false` na gênese, ativação é one-way.
    pub cap_enforced: bool,
    // --- bloco de estado da F2: campos NOVOS vao no fim, antes dos bumps ---
    /// DV12 — hash de atestação do NAV corrente, **em estado**.
    ///
    /// Já ia no evento `NavPublished`, e evento some da janela de retenção do
    /// RPC: backend que reindexa do zero seis meses depois não reconstrói qual
    /// atestação sustentava qual NAV. Em estado, sustenta.
    pub nav_attestation: [u8; 32],
    /// DV10 — carimbo da última **distribuição de lucro**. `0` = nunca
    /// distribuiu, e a primeira não espera cadência nenhuma.
    ///
    /// Renomeado de `last_accrue_ts` (D-F2-09): mesmo significado, mesmo lugar,
    /// mesmos bytes — o que morreu foi a instrução, não o conceito. Renome não
    /// move byte, então a conta viva não sente.
    ///
    /// **Também é a identidade da janela.** É estritamente crescente (a trava
    /// de cadência garante), então serve sozinho de "que distribuição é esta"
    /// para a marca de saque por carteira — sem contador de ciclo separado.
    pub ultima_distribuicao_ts: i64,
    /// Capital em campo: `deploy_capital` menos `return_capital`.
    ///
    /// Mesma lição da DV12 — o backend contabilizaria por evento, e evento
    /// expira. Soma corrente em estado é o que sobrevive a uma reindexação.
    pub deployed_usdc: u64,
    // --- bloco do resgate de capital (2026-08-25) ---
    /// Contador monotônico que semeia a PDA de cada pedido: `[RESGATE_SEED, id]`.
    ///
    /// Só cresce. Nunca é reusado nem decrementado, nem quando um pedido é pago —
    /// id reciclado permitiria recriar a PDA de um pedido já quitado.
    pub proximo_pedido_id: u64,
    /// Quantos pedidos venceram o prazo de 180 dias sem pagamento.
    ///
    /// Enquanto for maior que zero, `deploy_capital` recusa. É o dente da decisão
    /// de que o prazo é obrigação firme: não se aumenta a aposta devendo a quem
    /// pediu para sair. Sobe por `marcar_vencido`, que é **permissionless** — o
    /// próprio investidor prejudicado aciona, sem depender da mesa.
    pub resgates_em_atraso: u32,
    /// Conta de USDC de onde o resgate de capital é pago.
    ///
    /// A mesa a abastece **de fora** do cofre (empréstimo novo, saque de
    /// empréstimo, lucro de trading) conforme a saúde das pernas. O contrato não
    /// lê health factor, não toma empréstimo e não desmonta posição: ele confere
    /// se esta conta tem saldo e paga. Toda a inteligência é da mesa.
    ///
    /// Tem de ser conta cuja autoridade é a `authority` — o Vault do Squads —,
    /// igual à `origem_usdc` do `deposit_especial`: assim o pagamento é assinado
    /// pelo mesmo quórum que o aprovou, e não se introduz confiança nova.
    pub endereco_resgate: Pubkey,
    /// Destinos operacionais permitidos ao `deploy_capital`. Guarda a
    /// **carteira** (owner), nunca a ATA: o token define a ATA, e allowlist de
    /// ATA quebra a cada mint novo e não sobrevive a conta recriada.
    ///
    /// `Pubkey::default()` é vaga livre — e nunca casa, porque destino
    /// zerado não tem como ser owner de conta de token válida.
    pub deploy_allowlist: [Pubkey; DEPLOY_ALLOWLIST_LEN],
    pub bump: u8,
    pub escrow_bump: u8,
    pub treasury_bump: u8,
    // -----------------------------------------------------------------------
    // ATENÇÃO: campo novo vai **NO FIM**, sempre.
    // -----------------------------------------------------------------------
    // Borsh serializa na ordem de declaração. Campo inserido no meio muda o
    // significado de todos os bytes seguintes, e uma conta já gravada passa a
    // ser lida errada — sem erro, com números trocados. No fim, os bytes
    // antigos continuam valendo o que valiam e os novos entram depois deles,
    // que é o que torna a migração por `realloc` possível.
    // -----------------------------------------------------------------------
    /// Depósito mínimo em USDC, **em vigor**. Nasce em `MIN_DEPOSIT` e muda por
    /// `update_min_deposit` — proposta, não upgrade.
    ///
    /// É parâmetro operacional: a mesa começa alto para migrar investidor sem
    /// fuga de capital e baixa em etapas conforme democratiza. Fazer disso uma
    /// constante custaria um redeploy por degrau.
    pub min_deposit: u64,
    // -----------------------------------------------------------------------
    // Bloco do rendimento (D-F2-09). **No fim**, como manda o aviso acima.
    // -----------------------------------------------------------------------
    /// Quanto de USDC por cota a **última** distribuição gerou, na escala do
    /// NAV. É o `nav_depois - nav_antes` daquela distribuição, congelado.
    ///
    /// Congelado, e não recalculado a partir do NAV corrente, de propósito: o
    /// oráculo continua publicando durante a janela de saque, e um NAV novo no
    /// meio dela faria quem sacasse primeiro levar mais do que o seu. Com o
    /// delta preso, a ordem de chegada não muda o direito de ninguém.
    ///
    /// O direito de cada cotista é `cotas × delta / NAV_SCALE`, e a soma sobre
    /// quem já estava dentro dá exatamente a parcela dos cotistas.
    pub delta_lucro_por_cota: u64,
    /// Teto do bolo de saque de lucro ainda não pago, em USDC.
    ///
    /// Nasce em `40% de P` no `deposit_especial`, desce a cada saque e **zera
    /// quando o capital volta a campo** (`deploy_capital`) — é o re-deploy do
    /// ciclo seguinte que fecha a janela, não um cronômetro (D-F2-09).
    ///
    /// Faz três serviços num campo só:
    ///  1. `> 0` **é** a janela aberta — quem lê isto sabe se dá para sacar;
    ///  2. teto duro: as cotas que os sócios acabaram de receber existem na
    ///     hora do saque e, sem teto, reivindicariam mais do que o bolo tem;
    ///  3. trava de entrada: `deposit` e transferência de cota recusam
    ///     enquanto ele for `> 0`, senão quem entra na janela leva lucro que
    ///     não gerou.
    pub lucro_sacavel_restante: u64,
    // -----------------------------------------------------------------------
    // PARÂMETROS DE POLÍTICA — Upgrade E (D-F2-21)
    // -----------------------------------------------------------------------
    // Eram constantes no `.so`. Viraram campo pelo princípio que a mesa fechou
    // em 2026-09-08: **mudança de valor não deve exigir upgrade.**
    //
    // Oito dos onze entram com o valor que a constante já dizia. Três mudam de
    // valor na mesma operação, por decisão da mesa de 2026-09-08 — os pisos de
    // resgate (1.000 → 100) e de saque de lucro (200 → 100). É exceção
    // deliberada e o motivo está na `D-F2-22`: a mesa precisa dos pisos
    // baixos para mintar as primeiras cotas, e uma votação só existe **depois**
    // que este upgrade sobe. Não havia ordem em que a votação viesse antes.
    //
    // **Entram ANTES do `layout_version`**, que continua sendo o último campo —
    // é o que permite lê-lo em `data[len-2..len]` sem conhecer o resto.
    // -----------------------------------------------------------------------
    /// Prazo do resgate de capital, em segundos. **É TETO, não carência.**
    pub resgate_capital_prazo: i64,
    /// Piso do pedido de resgate, em USDC. Ver o OU em `constants.rs`.
    pub min_resgate_capital_usdc: u64,
    /// Piso do pedido de resgate, em COTAS. A trava anti-colapso do OU.
    pub min_resgate_capital_cotas: u64,
    /// Piso do saque de lucro, em USDC. Abaixo dele o saque **adia**, não perde.
    pub min_saque_lucro_usdc: u64,
    /// Idade máxima do NAV, em segundos. Acima, seis instruções param.
    pub max_nav_staleness: i64,
    /// Intervalo mínimo entre publicações **pelo oráculo**. A válvula não espera.
    pub min_nav_publish_interval: i64,
    /// Cadência mínima entre distribuições de lucro, em segundos.
    pub distribuicao_interval: i64,
    /// Variação máxima do NAV por publicação do oráculo, **em porcento**.
    ///
    /// Só o numerador vira campo: o denominador segue a constante 100. Campo
    /// para o denominador seria um caminho para gravar zero e dividir por ele.
    pub nav_bound_pct: u16,
    /// Teto de concentração por carteira, **em porcento**. Mesma razão do acima.
    pub cap_pct: u16,
    /// Taxa de performance **TOTAL** do ciclo, em pontos-base. 5.000 = 50%.
    ///
    /// A base é `P` — o lucro que voltou ao caixa pelo `deposit_especial` —, e
    /// a parcela dos cotistas é o que sobra: `P − 3 × parcela`. Por isso este
    /// campo é o único da lista com teto no binário
    /// (`MAX_PERF_FEE_BPS_POR_SOCIO`, 2.500 = 25% cada, 75% no total): sem ele,
    /// uma proposta da mesa poderia zerar a parcela dos cotistas sem que
    /// nenhum cotista votasse. O teto é a parte que a votação não alcança.
    pub perf_fee_bps_total: u16,
    // -----------------------------------------------------------------------
    // Upgrade J (D-F2-43) — o lucro realizado por índice, e o piso.
    //
    // A gaveta é a ATA de USDC do vault 1 do Squads: tudo que a mesa manda para
    // lá é lucro realizado do ciclo (P). O contrato acompanha o saldo dela e,
    // a cada USDC novo, sobe `indice_p` em `delta × INDICE_SCALE ÷ supply`.
    // Cada carteira guarda o índice em que entrou (`PosicaoDoCotista`); o
    // ganho dela no ciclo é `cotas × (indice_p − max(entrada, indice_ciclo))`.
    // Quem entra no meio paga o bruto (já com o P dentro) e não participa do
    // P anterior. Σ ganhos = P ao centavo, provado nos testes com 18 carteiras.
    // -----------------------------------------------------------------------
    /// A gaveta: conta de USDC cuja autoridade é o vault 1 do Squads. Gravada
    /// na migração; muda por `set_gaveta_usdc` (autoridade, proposta 2/3).
    pub gaveta_usdc: Pubkey,
    /// P acumulado por cota desde a gênese, escalado por `INDICE_SCALE`. Só sobe.
    pub indice_p: u128,
    /// `indice_p` no último fechamento — a entrada efetiva mínima do ciclo em curso.
    pub indice_ciclo: u128,
    /// `indice_ciclo` do fechamento anterior — a janela de saque calcula sobre
    /// `indice_ciclo − max(entrada, indice_ciclo_anterior)`.
    pub indice_ciclo_anterior: u128,
    /// Último saldo da gaveta que o contrato viu. Delta positivo vira índice;
    /// delta negativo recusa a instrução (`GavetaDiminuiu`). Zera no fechamento.
    pub gaveta_saldo_visto: u64,
    /// Σ dos deltas positivos da gaveta no ciclo — a prova de conservação do
    /// fechamento (`p_ciclo == saldo da gaveta`). Zera no fechamento.
    pub p_ciclo: u64,
    /// NAV líquido: (capital no campo + P na gaveta) ÷ supply. Só sobe dentro
    /// do ciclo; `publish_nav` recusa bruto abaixo dele. Recalculado no fechamento.
    pub nav_piso: u64,
    /// NAV gravado no fechamento — o preço da queima na janela de saque.
    pub nav_fechamento: u64,
    /// J7 — quanto a mesa ja' cobrou POR COTA por diluicao, acumulado desde a
    /// genese (micro-USDC por cota, escalado por INDICE_SCALE). Monotonico:
    /// a cada fechamento `+= mesa × INDICE_SCALE ÷ supply_antes_da_cunhagem`.
    /// O acerto de cada carteira compara o que ela pagou por diluicao com o
    /// que devia pelo indice (ganho × taxa_mesa) e cunha/queima a diferenca.
    pub indice_diluicao: u128,
    /// **Versão do layout desta conta. É, e continua sendo, o ÚLTIMO campo.**
    ///
    /// O discriminador do Anchor identifica **tipo**, não **versão**. Duas versões
    /// do mesmo struct com o **mesmo tamanho** — o que acontece quando uma
    /// migração compensa remoção com adição — têm discriminador igual, tamanho
    /// igual, e o `Account<Vault>` **desserializa a errada em silêncio**, com os
    /// campos trocados. Está provado em
    /// `tests/estado_f2.rs::layout_do_mesmo_tamanho_desserializa_e_so_uma_trava_acidental_barra`:
    /// o que barrou lá foi o `bump` deslocado quebrar a derivação da PDA, uma
    /// trava acidental que depende do valor que calhou de cair no byte.
    ///
    /// **Ser o último campo não é estilo, é o que faz o campo funcionar.** Assim
    /// ele se lê em `data[len-2..len]` — sem precisar conhecer o layout. Um campo
    /// de versão em offset fixo no meio seria circular: para achá-lo você já
    /// precisaria saber qual layout está lendo.
    ///
    /// **Campo novo entra ANTES deste.** Contraria o hábito de append-only do
    /// Borsh, e é de propósito.
    pub layout_version: u16,
}

/// Uma linha da lista de afiliados do fechamento (J1, D-F2-43 §1). Argumento
/// do `deposit_especial`, votado na proposta: `indicado` é o cotista, `afiliado`
/// quem o trouxe, `bps` a fatia **da parte da mesa** sobre o ganho do indicado
/// (500 = 5%). A comissão sai de dentro da mesa; o cotista não paga nada a mais.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AfiliadoDoFechamento {
    pub indicado: Pubkey,
    pub afiliado: Pubkey,
    pub bps: u16,
}

/// A posição de uma carteira no índice de P. PDA `[POSICAO_SEED, owner]`.
///
/// `indice_entrada` é a média ponderada pelas cotas dos índices em que cada
/// lote entrou (aporte, `deposit_para`, transferência recebida). Resgate não a
/// altera. Criada no primeiro aporte; carteira sem esta conta no fechamento é
/// carteira de antes do J — a migração cria as vinte de 21/09 com entrada 0.
#[account]
#[derive(InitSpace)]
pub struct PosicaoDoCotista {
    pub owner: Pubkey,
    pub indice_entrada: u128,
    /// J7 — o `indice_diluicao` do cofre no ultimo acerto desta carteira.
    pub indice_diluicao_visto: u128,
    /// J7 — o `indice_ciclo` ate onde a taxa desta carteira ja' foi acertada.
    pub indice_p_acertado: u128,
    pub bump: u8,
}

/// Marca de saque de lucro de uma carteira. PDA `[LUCRO_SEED, owner]`.
///
/// Existe por um motivo só, e é aritmético: o direito ao saque é
/// `cotas × delta`, e pagar não zera `cotas`. Sem marca, a mesma carteira saca
/// de novo — 196,36 na primeira chamada, 189,47 na segunda, 182,83 na terceira.
/// Cinco chamadas e sai com 4,7× o devido.
///
/// Guarda o `ultima_distribuicao_ts` do último saque. Como esse carimbo é
/// estritamente crescente, `marca < vault.ultima_distribuicao_ts` é
/// "ainda não sacou **desta** distribuição" sem precisar de contador de ciclo.
///
/// Criada no primeiro saque, `init_if_needed`. Quem nunca saca não paga rent.
#[account]
#[derive(InitSpace)]
pub struct LucroSacado {
    pub owner: Pubkey,
    pub ultima_distribuicao_sacada: i64,
    pub bump: u8,
}

/// Uma carteira habilitada. PDA `[WHITELIST_SEED, owner]`.
///
/// A ausência da conta é rejeição, nunca omissão (D5). Por isso o hook não usa
/// `Option` nem trata "conta vazia" como neutra.
/// **O porteiro da whitelist — `D-F2-34`.** PDA unica `[WL_OPERATOR_SEED]`.
///
/// Aprovar cotista era uma proposta 2/3 POR PESSOA. O gargalo nunca foi a
/// decisao — a mesa ja' decide na fila do painel —, era a assinatura de hardware
/// para executar uma decisao ja' tomada. Com uma lista de espera, isso vira o
/// que trava a captacao.
///
/// ⚠️ **O QUE ESTA CHAVE PODE, E O QUE ELA NAO PODE.**
///
/// Pode: inscrever e desinscrever carteiras, e definir o piso proprio de cada
/// uma. Nao pode: mover um centavo, mudar parametro, pausar, emitir cota,
/// atualizar o programa. **Ela abre a porta do fundo, e so.**
///
/// O que se perde e' real e vale dito: uma chave sozinha passa a poder deixar
/// entrar quem quiser, e a desinscrever quem ja' esta'. O que limita o estrago e'
/// que entrar no fundo exige APORTAR, e que a mesa nomeia e destitui o porteiro
/// por 2/3 — e cada ato dele fica na cadeia com a chave que assinou.
///
/// **Conta a parte, e nao campo no `Vault`.** Campo custaria realocar o cofre em
/// mainnet, e `Account<Vault>` falharia em toda instrucao ate' a migracao rodar.
#[account]
#[derive(InitSpace)]
pub struct WhitelistOperator {
    /// Quem aprova. Trocar e' ato da mesa.
    pub operator: Pubkey,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct WhitelistEntry {
    pub owner: Pubkey,
    pub active: bool,
    pub bump: u8,
    /// **Piso de aporte SO' DESTA CARTEIRA, em USDC. `0` = usa o do cofre.**
    ///
    /// Entrou no Upgrade G (`D-F2-30`). O `deposit` ja' carregava esta conta —
    /// ela e' a prova de que a carteira esta' aprovada —, entao o dado chega ao
    /// handler de graca: nenhuma conta nova, nenhuma leitura a mais.
    ///
    /// `0` significando "usa o padrao" evita `Option` e mantem o byte a byte
    /// simples. E' tambem o que faz as entradas ANTIGAS, de 42 bytes, lerem como
    /// "sem excecao" sem precisarem ser tocadas — ver `whitelist.rs`.
    ///
    /// **E' mecanismo, nao politica.** A mesa opera por excecao desde o primeiro
    /// dia; se um dia houver regra automatica ("quem ja' e' cotista aporta a
    /// partir de X"), ela nasce EM CIMA deste campo. A ordem importa e a razao e'
    /// assimetrica: o campo sem a regra serve; a regra sem o campo nao existe.
    pub min_deposit_proprio: u64,
}

/// Quanto de `fee_share` um sócio ainda não resgatou. PDA
/// `[FEE_SHARE_SEED, socio]`.
///
/// Existe porque `fee_share` e cota comum são o mesmo mint e o saldo do token
/// account é um número só (D3.1). Sem este livro não há como distinguir a cota
/// que veio de taxa — e é essa distinção que autoriza o resgate instantâneo.
#[account]
#[derive(InitSpace)]
pub struct FeeShareLedger {
    pub socio: Pubkey,
    pub shares: u64,
    pub bump: u8,
}

/// Um pedido de resgate de capital. PDA `[RESGATE_SEED, id_le_bytes]`.
///
/// **Uma conta por pedido, e não um vetor no cofre.** A F2 tinha fila de 32
/// posições porque havia um pedido por carteira (D-F2-13). Com resgate parcial
/// permitido — N pedidos por carteira, cada fração com sua data e seu pior-NAV —
/// qualquer teto de vetor vira o próximo bug, e vetor dentro do cofre faria o
/// rent de um pedido depender de quantos outros existem. Aqui cada pedido paga o
/// próprio rent e não há recurso comum a esgotar.
///
/// O dashboard enumera os pedidos de uma carteira por `getProgramAccounts` com
/// `memcmp` no `owner`. Funciona porque é o **nosso** programa — o Token Program,
/// que está fora dos índices secundários do RPC, não entra nessa consulta.
#[account]
#[derive(InitSpace)]
pub struct PedidoResgate {
    /// Quem pediu. Só ele recebe o pagamento.
    pub owner: Pubkey,
    /// O `proximo_pedido_id` do cofre no instante do pedido. Semeia a PDA.
    pub id: u64,
    /// A fração travada, em cotas. Estas cotas estão em `escrow_dom` e serão
    /// **queimadas** na efetivação — não devolvidas.
    pub cotas: u64,
    /// Quando foi pedido. Início do período do pior-NAV.
    pub solicitado_ts: i64,
    /// `solicitado_ts + RESGATE_CAPITAL_PRAZO`. **Teto, não carência:** a mesa
    /// pode efetivar antes. Passado sem pagamento, o pedido pode ser marcado em
    /// atraso e o cofre para de mandar capital para campo.
    pub vence_ts: i64,
    /// O `vault.nav` no instante do pedido. Serve de **teto** do pior-NAV.
    ///
    /// Gravado sem exigir NAV fresco, e isso é deliberado: a D-F2-03 fixou que
    /// **pedido de resgate é direito do cotista e não fica refém do backend**.
    /// Um NAV velho aqui produz um teto frouxo ou apertado, nunca um pagamento
    /// errado — quem paga é o `pior_nav` que a mesa informa, e ele tem outras
    /// três travas por cima.
    pub nav_na_solicitacao: u64,
    /// A catraca: o **menor** NAV já carimbado neste pedido.
    ///
    /// Nasce igual ao `nav_na_solicitacao` e só desce, por `carimbar_nav`, que é
    /// **permissionless** — o backend chama a cada publicação, e o investidor
    /// pode chamar pelo próprio pedido.
    ///
    /// **O que ela garante, e o que não garante.** Ela é limite **superior** do
    /// pior-NAV: impede a mesa pagar acima de um NAV que comprovadamente
    /// ocorreu, o que protege quem **fica** de ver o fundo diluído por um resgate
    /// generoso. Ela **não** protege quem sai — para isso seria preciso provar
    /// que nenhum NAV menor ocorreu, e o contrato não tem o histórico. Quem sai é
    /// protegido por verificabilidade contra o log de `NavPublished`, não por
    /// trava. Está dito assim no desenho porque fingir o contrário seria pior.
    pub pior_nav_visto: u64,
    /// `0` aberto · `1` pago · `2` em atraso.
    pub estado: u8,
    pub bump: u8,
}
