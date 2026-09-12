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
