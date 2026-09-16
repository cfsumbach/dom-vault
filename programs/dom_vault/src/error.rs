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
    #[msg("Pedido de resgate abaixo do minimo")]
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
}
