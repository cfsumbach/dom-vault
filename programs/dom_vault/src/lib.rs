pub mod constants;
pub mod error;
pub mod events;
pub mod indice;
pub mod instructions;
pub mod math;
pub mod state;
pub mod whitelist;

use {
    anchor_lang::prelude::*, spl_discriminator::discriminator::SplDiscriminate,
    spl_transfer_hook_interface::instruction::ExecuteInstruction,
};

pub use {constants::*, events::*, instructions::*, state::*};

// Program ID da **F2**. A F1 nasceu em `96gvBt479u7MYKxEekQyq3fTQVDWr8dQVKsmZkqwEkn2`
// e esta CONGELADA como prova historica (dossie, build verificavel, ciclo em
// devnet) — nao recebe o codigo da F2. O keypair dela esta arquivado em
// `.keys/dom_vault-F1-CONGELADO-keypair.json`.
//
// O ID entra COMPILADO no `.so`: `declare_id!` alimenta `crate::ID`, e o hook
// confere `hook_program == Some(crate::ID)` a cada transferencia. Subir este
// binario sob outro endereco quebraria o hook em silencio.
declare_id!("2KRqqGA47Pg2ML7Q8WJyaVAmEL8DaRx1sKxqzpVyNnkg");

// -----------------------------------------------------------------------------
// CANAL DE SEGURANCA — embutido no binario (D-F2-23, Upgrade F)
// -----------------------------------------------------------------------------
// Quem achar uma falha neste programa hoje **nao tem para quem escrever**. A
// alternativa a um canal nao e' silencio: e' o achado virar post publico antes
// de a mesa saber que existe.
//
// O `security.txt` vive DENTRO do `.so`, entao qualquer um o le' direto da
// cadeia — nao depende de site no ar, de dominio renovado nem de a mesa
// continuar operando o mesmo canal de divulgacao. Enquanto o binario estiver
// instalado, o contato esta' publicado.
//
// **E' publico e permanente ate' o proximo upgrade.** Nao ponha aqui endereco
// pessoal de ninguem: use uma caixa dedicada, que sobrevive a troca de pessoa.
//
// O bloco sai do binario quando o crate e' usado como biblioteca
// (`no-entrypoint`), porque ali ele nao seria alcancavel e so' ocuparia espaco.
#[cfg(not(feature = "no-entrypoint"))]
solana_security_txt::security_txt! {
    name: "DOM - Fundo Tokenizado",
    project_url: "https://token.depthofmarket.com.br",
    // Endereco de FUNCAO, nunca de pessoa. Este campo e' compilado: so' sai por
    // upgrade com proposta 2/3, entao o que entrar aqui fica colado ao fundo —
    // colhido por robos e impossivel de aposentar sem uma operacao. Pessoa muda
    // de papel; `security@` sobrevive.
    contacts: "email:security@depthofmarket.com.br",
    // ⚠️ APONTA PARA REPOSITORIO QUE EXISTE. URL morta aqui e' pior que campo
    // ausente: e' promessa que 404, num campo que so' sai por outro upgrade.
    // O espelho e' GERADO por `scripts/espelho.sh` e reproduz este binario bit
    // a bit — e' o que faz a frase "confira o sha256" deixar de ser promessa.
    source_code: "https://github.com/cfsumbach/dom-vault",
    policy: "Reporte por e-mail antes de divulgar. A mesa confirma o recebimento \
             e combina prazo de correcao com quem reportou. O fundo opera sob \
             multisig 2/3: correcao exige proposta e quorum, entao prazo de \
             divulgacao coordenada leva isso em conta. Nao ha' programa de \
             recompensa formal.",
    preferred_languages: "pt,en"
}
// O `source_code` ENTROU no Upgrade G. A decisao que faltava — abrir o repo
// inteiro ou publicar um espelho so' com o que o build exige — foi tomada pela
// mesa em 11/09: espelho, GERADO por `scripts/espelho.sh`, nascendo sem
// historico para nao arrastar o dossie.
//
// Ele so' entrou depois do repositorio EXISTIR. Apontar para um privado, ou
// para um que nao existe, seria pior que omitir: quem tentasse conferir bateria
// num 404 e concluiria que o projeto finge transparencia — e o campo so' sai
// por outro upgrade.

/// Cofre DOM. O transfer hook mora aqui dentro, não em programa separado (D1):
/// ele precisa ler estado do cofre — whitelist, `cap_enforced`, isenção das
/// contas do programa — a cada transferência.
#[program]
pub mod dom_vault {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        nav_oracle: Pubkey,
        socios: [Pubkey; NUM_SOCIOS],
    ) -> Result<()> {
        instructions::initialize::handle_initialize(ctx, nav_oracle, socios)
    }

    /// Sócios são configuráveis — e a trava de distinção vale aqui igual ao
    /// `initialize`, senão a do `initialize` é contornável em dois passos (D10).
    pub fn update_socios(ctx: Context<UpdateSocios>, socios: [Pubkey; NUM_SOCIOS]) -> Result<()> {
        instructions::update_socios::handle_update_socios(ctx, socios)
    }

    /// O terceiro argumento entrou no Upgrade G (`D-F2-30`): piso de aporte
    /// so' desta carteira, `0` = usa o do cofre.
    /// Nomeia (ou destitui, com `Pubkey::default()`) o porteiro da whitelist.
    ///
    /// Delegar continua sendo ato da mesa, 2/3. O que sai da mesa e' o ato
    /// REPETIDO de aprovar cada cotista — `D-F2-34`.
    pub fn set_whitelist_operator(ctx: Context<SetWhitelistOperator>, novo: Pubkey) -> Result<()> {
        instructions::set_whitelist_operator::handle_set_whitelist_operator(ctx, novo)
    }

    pub fn update_whitelist(
        ctx: Context<UpdateWhitelist>,
        owner: Pubkey,
        active: bool,
        min_deposit_proprio: u64,
    ) -> Result<()> {
        instructions::update_whitelist::handle_update_whitelist(
            ctx,
            owner,
            active,
            min_deposit_proprio,
        )
    }

    /// **Aporte EM NOME DE OUTRA CARTEIRA** — o bônus da mesa numa assinatura.
    /// Upgrade G (`D-F2-30`). As DUAS pontas passam pela whitelist: `mint_to`
    /// nao dispara o hook, entao sem a conferencia do beneficiario isto seria
    /// porta para cunhar cota em qualquer carteira. Ver `deposit_para.rs`.
    pub fn deposit_para(ctx: Context<DepositPara>, usdc_amount: u64) -> Result<()> {
        instructions::deposit_para::handle_deposit_para(ctx, usdc_amount)
    }

    /// Aporte em USDC contra emissão de cotas ao NAV corrente.
    ///
    /// Não mexe no NAV: cota nova sai pelo preço corrente, então quem já estava
    /// dentro não é diluído pelo novato (T02).
    pub fn deposit(ctx: Context<Deposit>, usdc_amount: u64) -> Result<()> {
        instructions::deposit::handle_deposit(ctx, usdc_amount)
    }

    /// Publica NAV com hash de atestacao. Só o oráculo, e o relógio não anda
    /// para trás (T35/T36/T37).
    pub fn publish_nav(
        ctx: Context<PublishNav>,
        nav: u64,
        attestation_hash: [u8; 32],
        timestamp: i64,
    ) -> Result<()> {
        instructions::publish_nav::handle_publish_nav(ctx, nav, attestation_hash, timestamp)
    }

    /// **Solicitar resgate de capital.** Trava a fração de cotas no escrow e abre
    /// o pedido, com prazo de 180 dias como teto.
    ///
    /// Irrevogável — **não existe** instrução de cancelamento, e é assim de
    /// propósito. Parcial é permitido: cada pedido tem sua data e seu pior-NAV.
    ///
    /// Sucede o `request_redeem` da fila D+30, que saiu: um produto, não dois.
    pub fn solicitar_resgate_capital(
        ctx: Context<SolicitarResgateCapital>,
        cotas: u64,
    ) -> Result<()> {
        instructions::resgate_capital::handle_solicitar_resgate_capital(ctx, cotas)
    }

    /// **Carimba o NAV corrente na catraca de um pedido. Permissionless.**
    ///
    /// Só desce. Quem chama não escolhe o número — ele vem do `vault.nav` do
    /// instante —, então deixar aberto não dá poder a ninguém. O backend chama a
    /// cada publicação; o investidor chama o próprio pedido se desconfiar.
    pub fn carimbar_nav(ctx: Context<CarimbarNav>) -> Result<()> {
        instructions::resgate_capital::handle_carimbar_nav(ctx)
    }

    /// **Marca um pedido vencido e trava o `deploy_capital`. Permissionless.**
    ///
    /// O prazo é obrigação firme. O contrato não obriga a mesa a desmontar
    /// posição, mas impede o cofre de mandar mais capital para campo enquanto
    /// dever a quem pediu para sair.
    pub fn marcar_vencido(ctx: Context<MarcarVencido>) -> Result<()> {
        instructions::resgate_capital::handle_marcar_vencido(ctx)
    }

    /// **Nomeia a conta de onde os resgates de capital são pagos. Privilegiada.**
    pub fn update_endereco_resgate(ctx: Context<UpdateEnderecoResgate>) -> Result<()> {
        instructions::resgate_capital::handle_update_endereco_resgate(ctx)
    }

    /// Upgrade J7: acerta a taxa de uma carteira pelo indice (credito sem privilegio; debito com o dono).
    pub fn acertar(ctx: Context<Acertar>) -> Result<()> {
        instructions::acertar::handle_acertar(ctx)
    }

    /// Upgrade J: a mesa aponta a gaveta do lucro (conta de USDC do vault 1).
    /// **Privilegiada.** Só entre ciclos.
    pub fn set_gaveta_usdc(ctx: Context<SetGavetaUsdc>) -> Result<()> {
        instructions::gaveta::handle_set_gaveta_usdc(ctx)
    }

    /// Upgrade J: absorve o P novo da gaveta no indice, sem privilegio.
    pub fn sincronizar_gaveta(ctx: Context<SincronizarGaveta>) -> Result<()> {
        instructions::gaveta::handle_sincronizar_gaveta(ctx)
    }

    /// Upgrade J: cria a posicao de uma carteira da whitelist que ainda nao aportou.
    pub fn abrir_posicao(ctx: Context<AbrirPosicao>) -> Result<()> {
        instructions::gaveta::handle_abrir_posicao(ctx)
    }

    /// Upgrade J: a lista de contas extras do hook ganha a posicao do destino. TEMPORARIA.
    pub fn atualizar_extra_account_meta_list(
        ctx: Context<AtualizarExtraAccountMetaList>,
    ) -> Result<()> {
        instructions::init_extra_account_metas::handle_atualizar_extra_account_meta_list(ctx)
    }

    /// Upgrade J (D-F2-43): migra o cofre 603 → 731 bytes; a gaveta nasce vazia e
    /// o `set_gaveta_usdc` a aponta na mesma proposta.
    /// TEMPORARIA — sai no upgrade seguinte, como a do E saiu no F.
    pub fn migrar_vault_indice(ctx: Context<MigrarVaultIndice>) -> Result<()> {
        instructions::migracao_j::handle_migrar_vault_indice(ctx)
    }

    /// **Rotaciona a chave do oráculo de NAV. Privilegiada.**
    ///
    /// A revogação que não existia: sem ela, chave vazada só se resolvia com
    /// upgrade de emergência, e a mesa perdia a corrida de publicação.
    pub fn set_nav_oracle(ctx: Context<SetNavOracle>, novo: Pubkey) -> Result<()> {
        instructions::publish_nav::handle_set_nav_oracle(ctx, novo)
    }

    /// **Efetiva um resgate de capital: queima a cota e paga. Privilegiada.**
    ///
    /// Paga do `endereco_resgate`, que a mesa abastece de fora do cofre — o
    /// treasury não é tocado. O `pior_nav` é informado pela mesa e limitado por
    /// cima três vezes; o contrato não tem histórico para provar que é o mínimo
    /// do período, e quem prova é quem lê o log de `NavPublished`.
    pub fn efetivar_resgate_capital(
        ctx: Context<EfetivarResgateCapital>,
        pior_nav: u64,
    ) -> Result<()> {
        instructions::resgate_capital::handle_efetivar_resgate_capital(ctx, pior_nav)
    }

    /// **Deposit especial** — o lucro realizado do ciclo entra no cofre e se
    /// reparte 60/40 na mesma transação. **Privilegiada.**
    ///
    /// Sucede o `accrue_performance` (D-F2-09). A base da taxa deixou de ser
    /// "o que passou do high water mark" e passou a ser `P`, o valor
    /// transferido aqui: taxa só sobre lucro que virou caixa, nunca sobre
    /// marcação. Abre a janela de saque de lucro.
    pub fn deposit_especial<'info>(
        ctx: Context<'info, DepositEspecial<'info>>,
        lucro_realizado: u64,
        afiliados: Vec<state::AfiliadoDoFechamento>,
    ) -> Result<()> {
        instructions::deposit_especial::handle_deposit_especial(ctx, lucro_realizado, afiliados)
    }

    /// Saque do lucro da janela, pelo cotista. **Não é privilegiada** — quem
    /// assina é o dono das cotas, como no `redeem_fee_share`.
    ///
    /// Queima `valor / nav` cotas e paga `cotas × delta` em USDC: o cotista
    /// leva o lucro e volta à posição em valor que tinha antes da distribuição.
    pub fn sacar_lucro(ctx: Context<SacarLucro>) -> Result<()> {
        instructions::sacar_lucro::handle_sacar_lucro(ctx)
    }

    /// Resgate de `fee_share`: instantâneo, contra caixa livre **acima da
    /// reserva**, sem passar pela fila D+30.
    pub fn redeem_fee_share(ctx: Context<RedeemFeeShare>, shares: u64) -> Result<()> {
        instructions::redeem_fee_share::handle_redeem_fee_share(ctx, shares)
    }

    /// Registra ou limpa um destino operacional da allowlist do
    /// `deploy_capital`. **Privilegiada** — decide para onde o capital dos
    /// cotistas pode sair (D-F2-01/D-F2-02).
    pub fn update_deploy_allowlist(
        ctx: Context<UpdateDeployAllowlist>,
        indice: u8,
        destino: Pubkey,
    ) -> Result<()> {
        instructions::update_deploy_allowlist::handle_update_deploy_allowlist(ctx, indice, destino)
    }

    /// Move USDC da treasury para um destino operacional allowlisted.
    /// **Privilegiada.** Três travas: autoridade, allowlist, e reserva + fila.
    pub fn deploy_capital(ctx: Context<DeployCapital>, valor: u64) -> Result<()> {
        instructions::deploy_capital::handle_deploy_capital(ctx, valor)
    }

    /// Devolve USDC para a treasury. **Não é privilegiada**: a allowlist
    /// controla por onde o dinheiro sai, entrar é irrestrito.
    pub fn return_capital(ctx: Context<ReturnCapital>, valor: u64) -> Result<()> {
        instructions::deploy_capital::handle_return_capital(ctx, valor)
    }

    /// Ajusta o depósito mínimo em vigor. **Privilegiada** — 12ª caneta.
    /// **Ajusta um parametro de politica. Privilegiada.** Upgrade E (D-F2-21).
    ///
    /// O valor sai do binario e passa a viver no estado: mudanca de valor deixa
    /// de exigir upgrade. Os LIMITES de cada parametro seguem constantes, e e' de
    /// proposito — se fossem votaveis, a protecao seria removivel pelo mesmo voto
    /// que ela protege.
    pub fn ajustar_parametro(
        ctx: Context<AjustarParametro>,
        qual: instructions::parametros::Parametro,
        novo: u64,
    ) -> Result<()> {
        instructions::parametros::handle_ajustar_parametro(ctx, qual, novo)
    }

    // A `corrigir_deployed_usdc` SAIU no Upgrade G (`D-F2-30`). Ela entrou no F
    // para zerar um `deployed_usdc` falso, cumpriu o servico na proposta #22 e
    // era temporaria por escrito — mesmo caminho da `migrar_vault_parametros`,
    // que saiu no F.
    //
    // Instrucao privilegiada que sobrevive ao proposito vira caneta esquecida:
    // ninguem a usa, ninguem a remove, e um dia alguem descobre que ela existe.
    // Os erros 6075/6076 FICAM declarados de proposito — remove-los renumeraria
    // o enum inteiro e quebraria toda tela que traduz codigo de erro.

    pub fn update_min_deposit(ctx: Context<UpdateMinDeposit>, novo: u64) -> Result<()> {
        instructions::min_deposit::handle_update_min_deposit(ctx, novo)
    }

    pub fn pause(ctx: Context<SetPause>) -> Result<()> {
        instructions::pause::handle_pause(ctx)
    }

    pub fn unpause(ctx: Context<SetPause>) -> Result<()> {
        instructions::pause::handle_unpause(ctx)
    }

    pub fn enable_cap(ctx: Context<EnableCap>) -> Result<()> {
        instructions::enable_cap::handle_enable_cap(ctx)
    }

    pub fn initialize_extra_account_meta_list(
        ctx: Context<InitializeExtraAccountMetaList>,
    ) -> Result<()> {
        instructions::init_extra_account_metas::handle_initialize_extra_account_meta_list(ctx)
    }

    /// `Execute` da transfer-hook-interface.
    ///
    /// O discriminador **não** é o do Anchor: é o da interface SPL, os 8
    /// primeiros bytes do hash de `"spl-transfer-hook-interface:execute"`. É o
    /// que o Token-2022 manda. Em Anchor 1.x o atributo `#[interface]` foi
    /// removido; o substituto é o discriminador customizado (A1 de
    /// `ANCHOR-1X-NOTAS.md`).
    #[instruction(discriminator = <ExecuteInstruction as SplDiscriminate>::SPL_DISCRIMINATOR_SLICE)]
    pub fn execute(ctx: Context<ExecuteHook>, amount: u64) -> Result<()> {
        instructions::execute::handle_execute(ctx, amount)
    }
}
