use {
    crate::{constants::NAV_SCALE, error::DomError},
    anchor_lang::prelude::*,
};

/// Cotas emitidas por um aporte em USDC, ao NAV informado.
///
/// `cotas = usdc * NAV_SCALE / nav`, em `u128`, **truncado**. O resto da divisão
/// fica no cofre: na dúvida o arredondamento é contra o usuário e a favor do
/// cofre (D9/T52). Emitir a cota "arredondada para cima" seria diluir quem já
/// está dentro por causa de uma fração.
///
/// NAV zero é erro, não divisão por zero: é a lição do `shares_base` do Drift
/// (T51), onde NAV próximo de zero inflava cota.
pub fn shares_from_usdc(usdc: u64, nav: u64) -> Result<u64> {
    require!(nav > 0, DomError::InvalidNav);

    let shares = (usdc as u128)
        .checked_mul(NAV_SCALE as u128)
        .ok_or(DomError::MathOverflow)?
        .checked_div(nav as u128)
        .ok_or(DomError::MathOverflow)?;

    u64::try_from(shares).map_err(|_| error!(DomError::MathOverflow))
}

/// Cotas que `usdc` compra ao NAV informado, **arredondado PARA CIMA**.
///
/// ─── ⚠️ QUANDO O CORRETO É PARA CIMA, E POR QUE O OUTRO ERA UM VAZAMENTO ────
///
/// A regra da casa é uma só: **o arredondamento é sempre a favor do cofre**
/// (D9/T52). O que muda é de que lado disso cada caminho está:
///
/// ```text
/// CUNHAR   cota para alguem  -> truncar  (emitir menos, nunca mais)
/// QUEIMAR  cota de alguem    -> PARA CIMA (queimar o suficiente, nunca de menos)
/// ```
///
/// `shares_from_usdc` trunca, e por isso serve à cunhagem. Usá-la para **queimar**
/// inverte o beneficiário: quem entrega menos cota do que o valor que recebe fica
/// com a fração, e a fração sai do preço da cota de quem ficou.
///
/// 🔴 **Medido pelo cc-fundo-devnet em 13/10, nos dois caminhos de queima:**
/// - **saque de lucro** (`sacar_lucro.rs`): 50 USDC ÷ 6,314952 = 7.917.716,56 cotas
///   → truncava para 7.917.716, que valem **49,999996** — o cotista levou **3,5
///   micro** a mais, e o teto é `⌊nav/10⁶⌋` ≈ **6 micro por saque**;
/// - **débito do acerto** (`indice.rs`): um débito menor que `⌊nav/10⁶⌋` virava
///   **zero cota**, não era cobrado — e a posição era marcada como acertada no
///   mesmo ato. ⚠️ Isso **perdoa a dívida para sempre**, e o teto **sobe com o
///   NAV**. A perda é da mesa.
///
/// ⓘ O inverso (`usdc_from_shares`) continua truncando: lá o cofre **paga**, e
/// truncar paga menos. Os dois lados do mesmo princípio.
pub fn shares_from_usdc_ceil(usdc: u64, nav: u64) -> Result<u64> {
    require!(nav > 0, DomError::InvalidNav);

    let numerador = (usdc as u128)
        .checked_mul(NAV_SCALE as u128)
        .ok_or(DomError::MathOverflow)?;
    let nav = nav as u128;
    // divisão para cima sem ponto flutuante: (a + b − 1) / b
    let shares = numerador
        .checked_add(nav - 1)
        .ok_or(DomError::MathOverflow)?
        .checked_div(nav)
        .ok_or(DomError::MathOverflow)?;

    u64::try_from(shares).map_err(|_| error!(DomError::MathOverflow))
}

/// USDC que um lote de cotas vale ao NAV informado.
///
/// `usdc = cotas * nav / NAV_SCALE`, em `u128`, **truncado** — inverso exato de
/// `shares_from_usdc`, com o arredondamento do mesmo lado: o do cofre (D9/T52).
/// É o que congela o `value` do pedido e o que mede o "pior valor" no
/// processamento (D8).
pub fn usdc_from_shares(shares: u64, nav: u64) -> Result<u64> {
    require!(nav > 0, DomError::InvalidNav);

    let usdc = (shares as u128)
        .checked_mul(nav as u128)
        .ok_or(DomError::MathOverflow)?
        .checked_div(NAV_SCALE as u128)
        .ok_or(DomError::MathOverflow)?;

    u64::try_from(usdc).map_err(|_| error!(DomError::MathOverflow))
}

/// DV11 — recusa **leitura** de NAV velho.
///
/// Aplicada em `deposit`, `request_redeem`, `process_redemptions` e
/// `accrue_performance`: os quatro pontos onde o NAV vira dinheiro.
///
/// **`nav_ts == 0` passa.** É a gênese: o oráculo ainda não publicou e o NAV em
/// vigor é o de fundação (1,000000), que não é preço de oráculo — é preço de
/// partida. A trava vale a partir da primeira publicação, que é quando existe
/// um oráculo de cuja frescura depender. Antes disso não há o que estar velho.
///
/// Que a fila também pare com NAV velho é decisão consciente: `process` paga
/// pelo menor entre o congelado e o de agora, então NAV velho ali paga preço
/// errado. A válvula de proposta do `publish_nav` é o que impede isso de virar
/// fila travada — a mesa sempre consegue publicar.
/// O `max_staleness` vem do ESTADO desde o Upgrade E — a mesa vota. O helper
/// recebe em vez de ler a constante: função pura continua pura, e quem chama
/// passa o campo do cofre que já tem em mãos.
pub fn require_nav_fresco(nav_ts: i64, now: i64, max_staleness: i64) -> Result<()> {
    if nav_ts == 0 {
        return Ok(());
    }
    let idade = now.saturating_sub(nav_ts);
    require!(idade <= max_staleness, DomError::NavStale);
    Ok(())
}

/// DV11 — a variação está dentro do limite por publicação?
///
/// `|novo - anterior| * DEN <= anterior * NUM`, em `u128` para não estourar.
/// Multiplicação em vez de divisão: divisão truncaria e deixaria passar um
/// pouco mais que o limite em NAV pequeno.
/// O `bound_pct` vem do ESTADO desde o Upgrade E. Ver `require_nav_fresco`.
pub fn nav_dentro_do_bound(anterior: u64, novo: u64, bound_pct: u16) -> bool {
    let delta = (anterior.max(novo) - anterior.min(novo)) as u128;
    let teto = (anterior as u128).saturating_mul(bound_pct as u128);
    delta.saturating_mul(100) <= teto
}

#[cfg(test)]
mod tests {
    /// ⚠️ A REGRA DO ARREDONDAMENTO, NOS DOIS SENTIDOS (item 2 da ordem de 13/10:
    /// *"arredondamento sempre a favor do cofre: cunhagem e saque nunca passam 1
    /// raw do devido"*).
    ///
    /// ```text
    /// CUNHAR  -> shares_from_usdc       (trunca: emite MENOS, nunca mais)
    /// QUEIMAR -> shares_from_usdc_ceil  (para cima: queima O SUFICIENTE, nunca menos)
    /// PAGAR   -> usdc_from_shares       (trunca: paga MENOS, nunca mais)
    /// ```
    #[test]
    fn porta_o_arredondamento_e_sempre_a_favor_do_cofre() {
        // o caso medido pelo devnet em 13/10: 50 USDC ao nav_fechamento 6,314952
        let nav = 6_314_952u64;
        let valor = 50_000_000u64;

        let truncado = shares_from_usdc(valor, nav).unwrap();
        let para_cima = shares_from_usdc_ceil(valor, nav).unwrap();
        assert_eq!(truncado, 7_917_716, "o numero que a devnet mediu");
        assert_eq!(para_cima, 7_917_717, "e o de cima e UMA cota raw a mais");

        // ⚠️ O QUE CADA UM FAZ COM O DINHEIRO, na queima:
        let vale_truncado = usdc_from_shares(truncado, nav).unwrap();
        let vale_para_cima = usdc_from_shares(para_cima, nav).unwrap();
        assert!(
            vale_truncado < valor,
            "truncar a queima entrega menos cota do que o valor recebido: {vale_truncado} < {valor} — a fracao fica com quem saca"
        );
        assert!(
            vale_para_cima >= valor,
            "para cima, a cota entregue cobre o valor recebido: {vale_para_cima} >= {valor}"
        );
        // e o excesso do lado do cofre nunca passa de UM raw de cota em USDC
        let um_raw = usdc_from_shares(1, nav).unwrap();
        assert!(
            vale_para_cima - valor <= um_raw,
            "e nao passa de 1 raw de cota ({um_raw} micro): {} ",
            vale_para_cima - valor
        );
        // o que o cofre perdia por saque, com o teto que o devnet declarou
        let perda = valor - vale_truncado;
        assert!(
            perda <= (nav as u128 / 1_000_000) as u64 + 1,
            "a perda por saque era de ate ⌊nav/10⁶⌋ micro: {perda}"
        );
    }

    /// O `ceil` só sobe quando há resto — divisão exata não vira cota a mais.
    #[test]
    fn porta_o_ceil_nao_cobra_cota_extra_em_divisao_exata() {
        let nav = 2_000_000u64; // 2,0
                                // 10 USDC a nav 2,0 = exatamente 5 cotas
        assert_eq!(shares_from_usdc(10_000_000, nav).unwrap(), 5_000_000);
        assert_eq!(shares_from_usdc_ceil(10_000_000, nav).unwrap(), 5_000_000);
        // e com resto, sobe exatamente 1
        assert_eq!(
            shares_from_usdc_ceil(10_000_001, nav).unwrap(),
            shares_from_usdc(10_000_001, nav).unwrap() + 1
        );
    }

    /// ⚠️ O DÉBITO DO ACERTO QUE ERA PERDOADO: menor que `⌊nav/10⁶⌋` virava zero.
    #[test]
    fn porta_o_debito_pequeno_nao_vira_zero() {
        let nav = 6_314_952u64;
        // 1 micro-USDC de debito
        assert_eq!(
            shares_from_usdc(1, nav).unwrap(),
            0,
            "truncado, 1 micro de debito virava ZERO cota — e a posicao era marcada como acertada"
        );
        assert_eq!(
            shares_from_usdc_ceil(1, nav).unwrap(),
            1,
            "para cima, 1 micro cobra 1 cota raw: a divida nao se perdoa"
        );
        // o teto do que se perdoava, com este NAV
        let teto = nav / 1_000_000;
        assert_eq!(
            shares_from_usdc(teto, nav).unwrap(),
            0,
            "⌊nav/10⁶⌋ micro ainda virava zero"
        );
        assert!(
            shares_from_usdc_ceil(teto, nav).unwrap() > 0,
            "e agora cobra"
        );
    }

    /// ⓘ OS 19 MICRO DO FECHAMENTO #141 — a conta que o devnet pediu para
    /// explicar **ou** tratar como defeito. Explicação: é a regra funcionando.
    ///
    /// No #141, a soma cunhada aos sócios ficou **19 micro abaixo** de
    /// `parcela_socios` (354,384200 contra 354,384219). Isso é o truncamento de
    /// `shares_from_usdc` em cada sócio **mais** a sobra da divisão por 3 — e os
    /// 19 micro **ficam no preço da cota**, com os cotistas. A regra é "nem um raw
    /// a mais", não "igual".
    ///
    /// ⓘ **Este teste é aritmética pura e não reproduz o #141 inteiro:** no
    /// fechamento real cada sócio cunha `cotas_por_socio + crédito − débito` (o
    /// acerto J7 compensado na própria cunhagem), e o supply medido lá foi
    /// 56.118.273 cotas contra as 56.118.009 desta conta idealizada. O que se
    /// prova aqui é a **regra** e o **teto**, não o número daquela transação.
    #[test]
    fn porta_os_19_micro_do_141_sao_a_regra_e_nao_defeito() {
        let nav = 6_314_952u64; // o nav_fechamento do #141
        let parcela_socios = 354_384_219u64;
        let por_socio = parcela_socios / 3;
        let sobra_da_divisao = parcela_socios - por_socio * 3;
        let cotas_por_socio = shares_from_usdc(por_socio, nav).unwrap();
        let cunhado_em_usdc = usdc_from_shares(cotas_por_socio * 3, nav).unwrap();

        assert!(
            cunhado_em_usdc <= parcela_socios,
            "NEM UM RAW A MAIS: {cunhado_em_usdc} <= {parcela_socios}"
        );
        let faltam = parcela_socios - cunhado_em_usdc;
        // ⚠️ O TETO DA FALTA TEM **DUAS** TRUNCAGENS COMPOSTAS, e a minha primeira
        // versão desta asserção contava só uma — ela reprovou com
        // "a falta (19) cabe na sobra da divisao (0) mais 1 raw por socio (6 cada)",
        // ou seja, 19 > 18. O teste estava errado, não o código:
        //
        //   1ª truncagem: `shares_from_usdc` por sócio — perde até 1 raw de cota,
        //                 que vale `um_raw` micro;
        //   2ª truncagem: `usdc_from_shares` ao converter de volta — perde até
        //                 1 micro.
        //
        // Por sócio, então, o teto é `um_raw + 1`.
        let um_raw = usdc_from_shares(1, nav).unwrap();
        assert!(
            faltam <= sobra_da_divisao + 3 * (um_raw + 1),
            "a falta ({faltam}) cabe na sobra da divisao ({sobra_da_divisao}) mais (1 raw + 1 micro) por socio ({} cada)",
            um_raw + 1
        );
        assert!(
            faltam > 0,
            "e ela existe: e o que fica no preco da cota, com os cotistas"
        );
    }

    // Os valores de hoje, importados da constante — que agora e' so' o valor de
    // PARTIDA do campo (Upgrade E). O teste continua conferindo o comportamento
    // com 15% e 26h porque e' esse o valor que o cofre nasce tendo; a mesa pode
    // votar outro, e o que se testa aqui e' a aritmetica, nao a politica.
    use super::*;
    use crate::constants::{MAX_NAV_STALENESS, NAV_BOUND_NUM};

    #[test]
    fn nav_de_genese_e_um_para_um() {
        assert_eq!(
            shares_from_usdc(200_000_000, NAV_SCALE).unwrap(),
            200_000_000
        );
    }

    #[test]
    fn nav_acima_de_um_emite_menos_cotas() {
        // NAV 1,250000 -> 1.000 USDC compram 800 cotas.
        assert_eq!(
            shares_from_usdc(1_000_000_000, 1_250_000).unwrap(),
            800_000_000
        );
    }

    #[test]
    fn sobra_de_divisao_fica_no_cofre() {
        // NAV 3,000000: 1 USDC daria 0,333333... cotas. Trunca.
        assert_eq!(shares_from_usdc(1_000_000, 3_000_000).unwrap(), 333_333);
    }

    #[test]
    fn nav_zero_e_erro() {
        assert!(shares_from_usdc(1_000_000, 0).is_err());
        assert!(usdc_from_shares(1_000_000, 0).is_err());
    }

    #[test]
    fn bound_do_nav_deixa_passar_ate_quinze_por_cento() {
        // 1,000000 -> 1,150000 e' exatamente o limite: passa.
        assert!(nav_dentro_do_bound(
            1_000_000,
            1_150_000,
            NAV_BOUND_NUM as u16
        ));
        // Um micro-USDC acima nao passa.
        assert!(!nav_dentro_do_bound(
            1_000_000,
            1_150_001,
            NAV_BOUND_NUM as u16
        ));
        // Simetrico para baixo.
        assert!(nav_dentro_do_bound(
            1_000_000,
            850_000,
            NAV_BOUND_NUM as u16
        ));
        assert!(!nav_dentro_do_bound(
            1_000_000,
            849_999,
            NAV_BOUND_NUM as u16
        ));
    }

    #[test]
    fn bound_barra_o_vetor_do_dv11() {
        // O ataque do achado: oraculo comprometido publica NAV de po' de arroz
        // para mintar cota infinita no deposito seguinte.
        assert!(!nav_dentro_do_bound(1_000_000, 1, NAV_BOUND_NUM as u16));
        assert!(!nav_dentro_do_bound(
            1_000_000,
            u64::MAX / 2,
            NAV_BOUND_NUM as u16
        ));
    }

    #[test]
    fn staleness_conta_da_publicacao_e_a_genese_passa() {
        // Genese: nunca publicado, nao ha o que estar velho.
        assert!(require_nav_fresco(0, 1_000_000_000, MAX_NAV_STALENESS).is_ok());
        // Publicado agora.
        assert!(require_nav_fresco(1_000_000, 1_000_000, MAX_NAV_STALENESS).is_ok());
        // No limite das 26h.
        assert!(require_nav_fresco(1_000_000, 1_000_000 + 26 * 3600, MAX_NAV_STALENESS).is_ok());
        // Um segundo depois do limite.
        assert!(
            require_nav_fresco(1_000_000, 1_000_000 + 26 * 3600 + 1, MAX_NAV_STALENESS).is_err()
        );
    }

    #[test]
    fn ida_e_volta_nunca_devolve_mais_do_que_entrou() {
        // O par de funções não pode criar valor num round-trip: quem aporta e
        // resgata na sequência, ao mesmo NAV, sai com no máximo o que entrou.
        for nav in [1, 999_999, NAV_SCALE, 1_250_000, 3_000_000, 7_777_777] {
            for usdc in [200_000_000u64, 1_000_000, 1, 123_456_789] {
                let cotas = shares_from_usdc(usdc, nav).unwrap();
                let volta = usdc_from_shares(cotas, nav).unwrap();
                assert!(
                    volta <= usdc,
                    "nav={nav} usdc={usdc}: voltou {volta}, mais do que entrou"
                );
            }
        }
    }
}
