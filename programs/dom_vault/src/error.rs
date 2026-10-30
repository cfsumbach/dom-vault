use anchor_lang::prelude::*;

/// Enum único de erro do `dom_vault`, cobrindo cofre e hook.
///
/// Anchor 1.x não aceita mais de um bloco `#[error_code]` no mesmo programa
/// (A5 de `ANCHOR-1X-NOTAS.md`).
///
/// A ordem das variantes é o código on-chain (`6000 + índice`). **Não
/// reordenar** — os testes e as evidências de fase citam o número.
#[error_code]
pub enum DomError {
    #[msg("Mint nao e o mint do DOM")]
    UnknownMint,
    #[msg("Conta de token nao pertence ao mint do DOM")]
    TokenAccountMintMismatch,
    #[msg("Hook invocado fora de uma transferencia real")]
    NotTransferring,
    #[msg("Carteira sem whitelist ativa")]
    NotWhitelisted,
    #[msg("Endereco de whitelist nao confere com o PDA da carteira")]
    InvalidWhitelistAddress,
    #[msg("Operacao deixaria o destino acima de 25% do supply")]
    CapExceeded,
    #[msg("cap_enforced ja esta ativo — ativacao e one-way")]
    CapAlreadyEnforced,
    #[msg("Apenas a autoridade do cofre pode executar")]
    Unauthorized,
    #[msg("Mint nao esta sob o Token-2022")]
    MintNotToken2022,
    #[msg("Mint nao aponta o transfer hook para este programa")]
    TransferHookNotConfigured,
    #[msg("transfer_hook authority do mint nao e a autoridade do cofre")]
    TransferHookAuthorityMismatch,
    #[msg("Conta de validacao da interface em endereco divergente")]
    InvalidExtraAccountMetaListAddress,
    // --- bloco do deposit e da matriz de cap: variantes NOVAS vao no fim ---
    #[msg("mint authority do DOM nao e o PDA do cofre")]
    MintAuthorityMismatch,
    #[msg("freeze authority do DOM nao e o PDA do cofre")]
    FreezeAuthorityMismatch,
    #[msg("Mint de USDC nao e o aceito pelo cofre")]
    UnknownUsdcMint,
    #[msg("Conta de caixa nao e a do cofre")]
    InvalidTreasury,
    #[msg("Deposito abaixo do minimo vigente do cofre")]
    DepositBelowMinimum,
    #[msg("Aporte nao compra nem uma unidade de cota")]
    ZeroShares,
    #[msg("NAV invalido")]
    InvalidNav,
    #[msg("Overflow aritmetico")]
    MathOverflow,
    // --- bloco 4 (resgate, NAV, processamento): variantes NOVAS vao no fim ---
    #[msg("Apenas o oraculo de NAV pode publicar")]
    NotOracle,
    #[msg("Timestamp de NAV regressivo")]
    NavTimestampRegressive,
    #[msg("Timestamp de NAV no futuro")]
    NavTimestampInFuture,
    #[msg("Fila de resgates cheia")]
    QueueFull,
    #[msg("Pedido de resgate sem cotas")]
    ZeroSharesRequested,
    #[msg("A transferencia de cotas para o escrow nao precede este pedido")]
    EscrowTransferMissing,
    #[msg("Saldo do escrow nao cobre as cotas pendentes")]
    EscrowShortfall,
    #[msg("Conta de escrow de cotas divergente")]
    InvalidEscrowAccount,
    #[msg("Conta de USDC do beneficiario nao confere com o pedido")]
    BeneficiaryMismatch,
    #[msg("Conta de fila divergente")]
    InvalidQueue,
    // --- bloco 5 (apuracao, fee_share, pause): variantes NOVAS vao no fim ---
    #[msg("Socios precisam ser tres carteiras distintas")]
    DuplicateSocio,
    #[msg("Carteira de socio divergente da configurada")]
    SocioMismatch,
    #[msg("Cofre pausado")]
    Paused,
    #[msg("Cofre nao esta pausado")]
    NotPaused,
    #[msg("Sem fee_share suficiente no livro do socio")]
    InsufficientFeeShare,
    #[msg("Caixa livre acima da reserva nao cobre o resgate de fee_share")]
    NoFreeCash,
    #[msg("Supply zerado")]
    EmptySupply,
    // --- bloco de estado da F2: variantes NOVAS vao no fim ---
    #[msg("NAV velho demais para ser usado")]
    NavStale,
    #[msg("Variacao do NAV acima do limite por publicacao — exige proposta da mesa")]
    NavOutOfBounds,
    /// ⚠️ ITEM 5 DO UPGRADE N · **VARIANTE MORTA, E ELA FICA ONDE ESTA.**
    ///
    /// Nenhum `require!` deste programa a usa, e o backend tem zero ocorrencias
    /// dela — conferido por varredura em `programs/`, `app/`, `scripts/` e no
    /// `backend-nav`. A ordem do dono para o Upgrade N diz *"remoção do erro morto
    /// `RedeemBelowMinimum`"*, e o escopo aprovado fechou na **alternativa 5B**:
    /// ela é **declarada morta, não apagada**.
    ///
    /// O motivo é de dinheiro, não de estilo. Variante no meio do enum **carrega o
    /// número de todas as que vêm depois**: apagar esta (6039) desceria
    /// `TooManyRequests` de 6040 para 6039, e assim por diante até o fim — 50
    /// códigos renumerados. **Código de erro é o que o operador vê quando algo
    /// falha**, e o painel carrega IDLs antigos (`dom-dashboard/lib/moldes.ts`).
    /// Um operador lendo "6040" num painel velho veria o nome errado, e numa
    /// emergência isso é pior que um erro sem nome.
    ///
    /// ⚠️ **Então apagar custa 50 nomes errados para economizar uma linha.** Ela
    /// fica, com a mensagem dizendo o que é. O `t_n5_nenhum_codigo_de_erro_renumera`
    /// é o portão que impede alguém de "limpar" isto no futuro.
    #[msg("variante morta desde a F2 — nenhum require! a usa; o slot fica para nao renumerar os codigos abaixo")]
    RedeemBelowMinimum,
    #[msg("Carteira ja tem o maximo de pedidos na fila")]
    TooManyRequests,
    #[msg("Apuracao antes do intervalo minimo")]
    AccrueTooSoon,
    #[msg("Cofre sem NAV publicado — o oraculo tem que publicar antes do primeiro aporte")]
    NavNeverPublished,
    // --- bloco 3 da F2 (porta operacional): variantes NOVAS vao no fim ---
    #[msg("Destino nao esta na allowlist de deploy")]
    DestinationNotAllowed,
    #[msg("Saida deixaria o caixa abaixo da reserva mais a fila")]
    ReserveViolation,
    #[msg("Indice fora da allowlist de deploy")]
    AllowlistIndexOutOfRange,
    #[msg("Carteira de socio nao pode ser destino operacional")]
    SocioNaoPodeSerDestino,
    #[msg("Movimento de capital de valor zero")]
    ZeroDeploy,
    // --- minimos ajustaveis e correcoes: variantes NOVAS vao no fim ---
    #[msg("Pedido de resgate vale zero USDC ao NAV corrente")]
    ZeroRedeemValue,
    #[msg("Deposito minimo invalido")]
    InvalidMinDeposit,
    // As tres variantes de migracao abaixo ficam, embora a instrucao tenha
    // saido no Upgrade F. Codigo de erro NAO se remove: os numeros sao a
    // interface, e tirar um do meio desloca todos os seguintes — quem tem
    // 6060 mapeado passaria a ver outra coisa. Ficam ocupando o lugar.
    //
    // Nao e' esquecimento: remover variante do meio do enum **renumera todas as
    // seguintes**, e o codigo de erro e' contrato publico — o dashboard mapeia
    // numero para mensagem, e evidencia antiga cita numero. Trocar o significado
    // de `6052` para poupar tres linhas mortas seria estrago gratuito, ainda por
    // cima as vesperas da auditoria.
    #[msg("Cofre ja migrado — a conta ja tem o tamanho novo")]
    VaultAlreadyMigrated,
    #[msg("Conta do cofre com tamanho inesperado")]
    UnexpectedVaultSize,
    // --- bloco do rendimento (D-F2-09): variantes NOVAS vao no fim ---
    #[msg("Distribuicao de lucro antes do intervalo minimo")]
    DistribuicaoMuitoCedo,
    #[msg("Lucro realizado do ciclo tem que ser maior que zero")]
    ZeroLucro,
    #[msg("Janela de saque de lucro fechada — nao ha distribuicao aberta")]
    JanelaDeLucroFechada,
    #[msg("Esta carteira ja sacou o lucro desta distribuicao")]
    LucroJaSacado,
    #[msg("Socio saca pelo redeem_fee_share, nao pela janela dos cotistas")]
    SocioForaDaJanela,
    #[msg("Janela de saque de lucro aberta — entrada e transferencia de cota estao fechadas")]
    JanelaDeLucroAberta,
    #[msg("Carteira sem lucro a sacar nesta distribuicao")]
    SemLucroASacar,
    // --- bloco do resgate de capital: variantes NOVAS vao no fim ---
    #[msg("Resgate de capital abaixo do minimo")]
    ResgateAbaixoDoMinimo,
    #[msg("Pedido de resgate ja foi pago")]
    ResgateJaPago,
    #[msg("Pior NAV informado e maior que o teto do pedido")]
    PiorNavAcimaDoTeto,
    #[msg("Pior NAV informado tem que ser maior que zero")]
    PiorNavZero,
    #[msg("Endereco de resgate sem saldo para este pedido")]
    SemSaldoNoEnderecoDeResgate,
    #[msg("Pedido de resgate ainda nao venceu")]
    ResgateNaoVenceu,
    #[msg("Pedido de resgate ja esta marcado em atraso")]
    ResgateJaEmAtraso,
    #[msg("Ha resgate vencido nao pago — o cofre nao manda capital para campo")]
    ResgatesEmAtraso,
    #[msg("Layout da conta do cofre em versao inesperada")]
    LayoutVersaoInesperada,
    #[msg("Conta informada nao e' o endereco de resgate do cofre")]
    EnderecoDeResgateInvalido,
    #[msg("Saldo de cotas insuficiente para o resgate pedido")]
    CotasInsuficientes,
    #[msg("Fila de resgate nao esta vazia — migrar deixaria pedido sem porta")]
    FilaNaoVazia,
    #[msg("Publicacao de NAV pelo oraculo antes do intervalo minimo")]
    NavPublicacaoMuitoCedo,
    // Upgrade D. **No fim do enum de proposito**: variante inserida no meio
    // renumera todas as seguintes, e codigo de erro e' contrato publico — o
    // painel mapeia numero para mensagem. Acrescentar no fim nao mexe em nenhum
    // codigo ja' emitido.
    #[msg("Lucro do ciclo abaixo do minimo de saque")]
    SaqueLucroAbaixoDoMinimo,
    // Upgrade E. No fim do enum, pela mesma razao do 6072.
    #[msg("Valor fora do limite votavel para este parametro")]
    ParametroForaDoLimite,
    #[msg("Parametros incoerentes entre si — o intervalo tem de caber na validade")]
    ParametrosIncoerentes,
    // Upgrade F. No fim do enum, pela mesma razao do 6072 e do 6073.
    #[msg("O supply nao e' zero — a correcao so' roda em cofre vazio")]
    SupplyNaoZerado,
    #[msg("Nada a corrigir: o campo ja' esta' zerado")]
    NadaACorrigir,
    // Correcao do intervalo do oraculo. No fim do enum, pela mesma razao das
    // variantes acima: codigo de erro e' contrato publico.
    #[msg("Timestamp de NAV do oraculo atrasado demais em relacao ao relogio da rede")]
    NavTimestampMuitoAntigo,
    // Upgrade J (D-F2-43). No fim do enum, como sempre: codigo de erro e' contrato publico.
    #[msg("A gaveta tem menos do que o contrato viu — nunca se desconta o indice")]
    GavetaDiminuiu,
    #[msg("Conservacao falhou: p_ciclo nao bate com o saldo da gaveta")]
    PConservacaoFalhou,
    #[msg("A conta da gaveta nao e' a que o cofre conhece")]
    GavetaErrada,
    #[msg("A posicao do cotista no indice nao existe — carteira de antes do J sem migracao")]
    PosicaoDoCotistaAusente,
    #[msg("Acerto pendente: a carteira deve taxa e so' o dono pode queimar — chame `acertar` assinando")]
    AcertoPendente,
    #[msg("O P na gaveta nao e' o que a proposta esperava")]
    PDiferenteDoEsperado,
    #[msg("Lista de afiliados invalida: bps fora de 1..=500, indicado == afiliado, ou contas que nao batem com a lista")]
    AfiliadoInvalido,
    #[msg("Indicado repetido na lista de afiliados — uma comissao por indicado")]
    AfiliadoRepetido,
    #[msg("A conta de cota nao e' a ATA do dono — no J toda cota vive na ATA (o acerto e' por carteira, nao por conta)")]
    ContaDeCotaNaoEAta,

    /// Upgrade K (D-F2-45): `ajustar_deployed_usdc` só REDUZ. Aumentar levantaria
    /// o `nav_piso`, que desde o K é o preço mínimo de entrada — uma caneta que
    /// encarece a cota de quem chega.
    #[msg(
        "deployed_usdc so pode ser reduzido: aumentar levantaria o piso, que e' o preco de entrada"
    )]
    DeployedSoReduz,
    /// Upgrade K: correção privilegiada sem motivo escrito é caneta sem rastro.
    #[msg("o motivo do ajuste tem de ter entre 8 e 200 caracteres")]
    MotivoInvalido,
    /// Upgrade L: a gaveta tem MENOS que o número votado.
    ///
    /// ⚠️ Substitui o `PDiferenteDoEsperado` no fechamento, e a troca é a correção
    /// de uma negação de serviço. Exigir igualdade exata entre o voto e o saldo
    /// dava a qualquer um na rede o poder de derrubar a proposta mandando poeira
    /// entre o voto e a execução — por frações de centavo, indefinidamente.
    ///
    /// Medido em 05/10: quatro entradas de poeira na gaveta, de dois endereços
    /// *vanity* construídos sobre a Operacional do fundo, chegando 23 s e 5 s
    /// depois de transferências nossas.
    ///
    /// Agora a mesa vota um **piso**: o saldo tem de cobrir o voto, e o que se
    /// distribui é o voto. Excedente não decide nada — fica na gaveta como saldo
    /// não reconhecido.
    #[msg("a gaveta tem menos USDC do que o numero que a mesa votou")]
    GavetaAbaixoDoVotado,
    /// ⚠️ Upgrade N item 3: a conta `Multisig` do Squads não tem o layout que este
    /// contrato sabe ler.
    ///
    /// A pausa rápida lê a conta de **outro programa** para saber quem vota. Se o
    /// Squads mudar o layout num upgrade deles, ler por offset devolveria **lixo
    /// que parece permissão** — e lixo que parece permissão, numa instrução de
    /// pausa, é pior que não ter pausa rápida.
    ///
    /// Então o contrato confere, antes de ler membro: o dono da conta é o Squads ·
    /// o discriminador é o medido · a `Option<Pubkey>` do `rent_collector` tem tag
    /// 0 ou 1 · e o `Vec<Member>` cabe nos bytes da conta. Qualquer um dos quatro
    /// falhando, recusa aqui em vez de decidir.
    ///
    /// ⚠️ **Este erro é o último (6090), e entrou no fim do enum de propósito:**
    /// código de erro é o que o operador vê, e o painel carrega IDLs antigos.
    /// Inserir no meio deslocaria todos os códigos abaixo.
    #[msg("a conta do multisig do Squads nao tem o layout que este contrato sabe ler")]
    LayoutDoSquadsMudou,
    /// A conta passada como reserva de comissão de um afiliado não é a PDA
    /// `[COMISSAO_SEED, afiliado]`, ou não é gravável.
    ///
    /// ⚠️ **Entra no FIM do enum (6091), pelo mesmo motivo do 6090:** código de
    /// erro é o que o operador vê, e o painel carrega IDLs antigos. Inserir no
    /// meio deslocaria todos os códigos abaixo.
    #[msg("a conta da reserva de comissao do afiliado nao e a PDA esperada, ou nao e gravavel")]
    ReservaDeComissaoInvalida,
    /// O retrato do ciclo anterior ainda está **aberto**: há comissão do ciclo
    /// passado sem pagar, e fechar outro ciclo deixaria duas Fases B vivas ao
    /// mesmo tempo — alguém pagaria a comissão de um ciclo contra o retrato do
    /// outro. Decisão 4 do dono (16:55).
    #[msg("o retrato do ciclo anterior ainda esta aberto: termine a fase B dele antes de fechar outro ciclo")]
    RetratoAnteriorAberto,
    /// A conta passada como retrato não é a PDA `[RETRATO_SEED, ciclo]`, não é
    /// gravável, ou o ciclo votado não encadeia com o anterior.
    #[msg("a conta do retrato do ciclo nao e a PDA esperada, ou o ciclo votado nao encadeia")]
    RetratoDoCicloInvalido,
    /// A mesa votou orçamento de comissões **e** uma lista de afiliados na mesma
    /// proposta. Os dois caminhos existem em paralelo durante a migração para o
    /// fechamento em lotes, e são **mutuamente exclusivos**: ou a comissão é paga
    /// no próprio fechamento (o caminho antigo), ou ela fica em custódia para a
    /// Fase B pagar.
    #[msg("orcamento de comissoes e lista de afiliados na mesma proposta: escolha um dos dois caminhos")]
    OrcamentoComListaDeAfiliados,
    /// A mesa votou **linhas sem orçamento**: um retrato que diz "5 linhas a
    /// pagar" com nada para pagar. A Fase B nunca fecharia esse ciclo.
    ///
    /// ⚠️ Ele existe porque a mensagem do `OrcamentoComListaDeAfiliados` **mentia**
    /// nesse caso: ela manda "escolha um dos dois caminhos" para uma proposta que
    /// não tem lista nenhuma, e o operador vai procurar uma lista que não existe.
    /// Achado da revisão adversária de 13/10.
    #[msg("linhas votadas com orcamento ZERO: a fase B nao teria com que pagar")]
    LinhaVotadaSemOrcamento,
    /// A mesa votou **orçamento sem linha**: custódia de dinheiro com ninguém a
    /// receber. O retrato nasceria aberto e, pela decisão 7, a sobra só volta no
    /// lote que faz `linhas_pagas == linhas_votadas` — com zero linha nunca haveria
    /// lote, e o ciclo seguinte travaria em `RetratoAnteriorAberto`.
    #[msg("orcamento de comissoes sem linha votada: dinheiro em custodia sem ninguem a pagar")]
    OrcamentoSemLinhaVotada,
    /// O ciclo votado é anterior ao primeiro que nasce com retrato.
    #[msg("o ciclo votado e anterior ao primeiro ciclo com retrato")]
    CicloAntesDoPrimeiroRetrato,
    /// A soma da comissao paga no fechamento com o orcamento em custodia passa da
    /// parte da mesa. O canone da casa: comissao debitada da parte da mesa, e soma
    /// das comissoes <= parte da mesa.
    ///
    /// ⓘ A trava equivalente do caminho antigo (deposit_especial.rs:738) recusa com
    /// MathOverflow, que nao diz o que aconteceu. Nao mexo nela aqui — e outro
    /// escopo —, mas a nova nasce com erro proprio.
    #[msg("a comissao mais a custodia passam da parte da mesa")]
    CustodiaAcimaDaParteDaMesa,
    /// O lote veio sem nenhuma linha.
    #[msg("lote vazio: nada a pagar")]
    LoteVazio,
    /// O lote tem mais linhas do que `K_MAX_DO_LOTE`. ⚠️ Não é gosto: é o teto de
    /// travas da rede.
    #[msg("o lote passa do k maximo de linhas")]
    LoteAcimaDoK,
    /// Pagar este lote faria `linhas_pagas` passar de `linhas_votadas`. A contagem
    /// votada na Fase A é a trava, porque a lista não é congelada.
    #[msg("o lote pagaria mais linhas do que a mesa votou")]
    LinhasAcimaDoVotado,
    /// O número de contas não é exatamente o que o lote pede.
    #[msg("as contas do lote nao sao exatamente as que ele pede")]
    ContasDoLoteIncompletas,
    /// A conta passada como marca de pago não é a PDA `[PAGO_SEED, ciclo,
    /// indicado]`, ou não é gravável.
    #[msg("a marca de pago nao e a PDA esperada, ou nao e gravavel")]
    MarcaDePagoInvalida,
    /// Esta linha **já foi paga** neste ciclo: a marca existe.
    #[msg("a linha deste indicado ja foi paga neste ciclo")]
    LinhaJaPagaNesteCiclo,
    /// A soma dos lotes passaria do orçamento que a Fase A separou.
    #[msg("o lote passa do orcamento votado no fechamento")]
    LoteAcimaDoOrcamento,
    /// O retrato do ciclo está **fechado**: não há custódia a pagar.
    #[msg("o retrato deste ciclo esta fechado: nao ha comissao em custodia")]
    RetratoFechado,
    /// O afiliado é o próprio indicado.
    ///
    /// ⓘ O painel já conhece esta recusa — ela está na lista de selos que recusam
    /// (`PEDIDO-FUNDO-PARA-PAINEL-SELOS-QUE-RECUSAM-13-10.md`).
    #[msg("o afiliado nao pode ser o proprio indicado")]
    AfiliadoIgualIndicado,
    /// Duas linhas do mesmo **indicado** no mesmo lote: elas tentariam criar a
    /// mesma marca de pago, e a segunda falharia com o erro opaco do System
    /// Program em vez de dizer o que aconteceu.
    #[msg("duas linhas do mesmo indicado no mesmo lote")]
    IndicadoRepetidoNoLote,
    /// A conta de cota passada para a linha pertence a outra carteira que não o
    /// `afiliado` que a mesa votou **naquela linha**.
    ///
    /// ⚠️ Sem esta recusa, trocar a ORDEM das contas reatribuía o dinheiro entre
    /// afiliados com os mesmos dados de instrução — medido pela revisão adversária
    /// de 13/10: 200 USDC e 1 USDC trocando de dono.
    #[msg("a conta de cota da linha nao e do afiliado que a mesa votou nela")]
    AfiliadoDaLinhaDivergente,
    /// O ciclo declarado **não é o último**: o retrato de `ciclo + 1` já existe.
    ///
    /// ⚠️ Ele existe porque o `RetratoDoCicloInvalido` cobria sete condições
    /// diferentes, e duas delas são as que o operador mais vai ver — "você declarou
    /// o ciclo errado" e "o cofre está travado por layout" saíam idênticas no log.
    /// Achado da revisão adversária de 13/10.
    #[msg("o ciclo declarado nao e o ultimo: existe retrato mais novo")]
    CicloDeclaradoNaoEOUltimo,
    /// O retrato existe mas **não desserializa**: foi gravado por outra versão do
    /// programa. ⚠️ Não é recuperável por instrução — ver o doc de
    /// `reservas_em_aberto_micro`.
    #[msg("o retrato existe mas foi gravado por outra versao do programa")]
    RetratoDeOutroLayout,
    /// O retrato do ciclo **declarado** está aberto.
    ///
    /// ⓘ Distinto do `RetratoAnteriorAberto`: lá o retrato é o **anterior** ao que
    /// nasce; aqui é o que o chamador declarou ser o último. O nome do outro mentia
    /// no log do `update_socios`.
    #[msg("o retrato do ciclo declarado esta aberto")]
    RetratoDeclaradoAberto,
    /// As SEIS contas da mesa (3 ATAs de cota + 3 posições) não vieram no
    /// `remaining_accounts`. ⚠️ Recusa em vez de pagar a quem veio: a sobra do
    /// orçamento é dinheiro da mesa, e conta faltando mudaria o rateio.
    #[msg("as seis contas da mesa (3 cotas + 3 posicoes) nao vieram")]
    ContasDaMesaFaltando,
    /// A contagem não fecha: `linhas_nao_pagas + marcas_fechadas` tem de ser
    /// exatamente `linhas_votadas` do retrato.
    ///
    /// ⚠️ É esta recusa que faz o evento do `encerrar_ciclo` ser PROVA e não
    /// declaração: sem ela, a mesa poderia encerrar nomeando metade das linhas.
    #[msg("as linhas nao pagas mais as marcas fechadas nao somam o votado")]
    LinhasDoEncerramentoNaoFecham,
    /// A linha declarada como NÃO paga **tem** marca de pago neste ciclo.
    #[msg("a linha declarada nao paga tem marca de pago")]
    LinhaDeclaradaNaoPagaEstaPaga,
    /// A reserva de comissão está **vazia**: não há o que liquidar, e liquidar
    /// zero queimaria o rent do afiliado sem lhe dar cota.
    #[msg("a reserva de comissao esta vazia")]
    ReservaVazia,
    /// O retrato passado **não é o mais recente**: o que a Fase A gravou nele não
    /// bate com o cofre (`RetratoDoCiclo::e_o_corrente`). Também no `publish_nav`
    /// (3a), e lá cobre a conta vazia que não é a PDA do primeiro ciclo.
    ///
    /// ⚠️ O 7.6 decrementa o contador do retrato MAIS RECENTE (dono, 14/10 16:00).
    /// Sem esta recusa, decrementar um retrato velho deixaria o contador do atual
    /// alto — e o contador é o que o backend usa para descontar a dívida do NAV.
    #[msg("o retrato passado nao e o mais recente do cofre")]
    RetratoNaoEOMaisRecente,
    /// A conta de marca de pago passada não é a PDA que ela deveria ser — chave
    /// forjada, ciclo de outro retrato, ou dono que não é o programa.
    #[msg("a conta de marca de pago divergiu da PDA esperada")]
    ContaDeMarcaDivergente,
    /// O livro de cotas do sócio (`FeeShareLedger`) não é a PDA do sócio, ou é
    /// conta de outro dono ou de outro tipo. Era o `init_if_needed` do Anchor que recusava
    /// isso; agora é o corpo (DECISOES 19/10 17:05, bump gravado).
    #[msg("o livro de cotas do socio divergiu da PDA esperada")]
    LivroDoSocioInvalido,
}
