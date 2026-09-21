//! O índice de P por cota — Upgrade J, D-F2-43 §2.
//!
//! Três funções puras, sem conta e sem CPI, para as provas de unidade cobrarem
//! a aritmética antes de qualquer instrução usá-la:
//!
//!   `absorver_gaveta`  — o saldo lido da gaveta vira índice (delta ≥ 0) ou recusa
//!   `entrada_ponderada` — a média ponderada pelas cotas quando um lote novo entra
//!   `ganho`             — cotas × (índice de referência − entrada efetiva)
//!
//! Tudo em u128 escalado por `INDICE_SCALE` (1e18). `supply_raw` em 6 casas,
//! `micro` em 6 casas: `indice += delta_micro × 1e18 ÷ supply_raw`, então
//! `ganho_micro = cotas_raw × Δindice ÷ 1e18`. Cabe: supply ≤ 2^64, delta ≤
//! 2^64, 1e18 < 2^60 → produto < 2^124.

use {
    crate::{constants::INDICE_SCALE, error::DomError},
    anchor_lang::prelude::*,
};

/// O que uma leitura da gaveta produz.
#[derive(Debug, PartialEq, Eq)]
pub struct Absorcao {
    /// Quanto entrou de novo, em micro-USDC (0 se nada mudou).
    pub delta_micro: u64,
    /// O índice depois de absorver.
    pub indice_p: u128,
}

/// Lê o saldo da gaveta contra o que o contrato tinha visto.
///
/// - saldo == visto: nada a fazer;
/// - saldo > visto: o delta é P novo — sobe o índice com o supply DESTE instante;
/// - saldo < visto: **recusa**. Nunca se desconta o índice: se a gaveta encolheu,
///   alguém tirou P antes do fechamento (o caso de 20/09) e o cofre para até a
///   mesa explicar.
///
/// Com `supply_raw == 0` (cofre vazio, antes do primeiro aporte) o delta não tem
/// a quem pertencer: fica em `p_ciclo` e no saldo visto, e o índice não anda —
/// o primeiro aporte entra com o índice zerado e o P é dele.
pub fn absorver_gaveta(
    saldo_lido: u64,
    saldo_visto: u64,
    supply_raw: u64,
    indice_p: u128,
) -> Result<Absorcao> {
    require!(saldo_lido >= saldo_visto, DomError::GavetaDiminuiu);
    let delta_micro = saldo_lido - saldo_visto;
    if delta_micro == 0 || supply_raw == 0 {
        return Ok(Absorcao {
            delta_micro,
            indice_p,
        });
    }
    let passo = (delta_micro as u128)
        .checked_mul(INDICE_SCALE)
        .ok_or(DomError::MathOverflow)?
        .checked_div(supply_raw as u128)
        .ok_or(DomError::MathOverflow)?;
    Ok(Absorcao {
        delta_micro,
        indice_p: indice_p.checked_add(passo).ok_or(DomError::MathOverflow)?,
    })
}

/// A entrada de uma carteira depois de receber `cotas_novas` ao índice `indice_agora`.
///
/// Média ponderada pelas cotas: o lote antigo continua valendo a entrada antiga,
/// o lote novo entra no índice de agora. É o que faz "aporte adicional" e
/// "transferência recebida" não ganharem P anterior a elas — e o que faz um
/// aporte com o índice parado não mudar o sacável de quem já estava.
pub fn entrada_ponderada(
    cotas_antigas: u64,
    entrada_antiga: u128,
    cotas_novas: u64,
    indice_agora: u128,
) -> Result<u128> {
    let total = (cotas_antigas as u128)
        .checked_add(cotas_novas as u128)
        .ok_or(DomError::MathOverflow)?;
    if total == 0 {
        return Ok(indice_agora);
    }
    let soma = (cotas_antigas as u128)
        .checked_mul(entrada_antiga)
        .ok_or(DomError::MathOverflow)?
        .checked_add(
            (cotas_novas as u128)
                .checked_mul(indice_agora)
                .ok_or(DomError::MathOverflow)?,
        )
        .ok_or(DomError::MathOverflow)?;
    Ok(soma / total)
}

/// O ganho de uma carteira entre `entrada` e `referencia`, em micro-USDC.
///
/// `entrada_efetiva = max(entrada, piso_do_ciclo)`: quem entrou antes do último
/// fechamento conta a partir dele. Referência abaixo da entrada (carteira que
/// entrou depois do fechamento, olhando a janela anterior) dá zero, nunca
/// negativo.
pub fn ganho(cotas_raw: u64, entrada: u128, piso_do_ciclo: u128, referencia: u128) -> Result<u64> {
    let efetiva = entrada.max(piso_do_ciclo);
    if referencia <= efetiva {
        return Ok(0);
    }
    let bruto = (cotas_raw as u128)
        .checked_mul(referencia - efetiva)
        .ok_or(DomError::MathOverflow)?
        / INDICE_SCALE;
    u64::try_from(bruto).map_err(|_| error!(DomError::MathOverflow))
}

#[cfg(test)]
mod provas {
    use super::*;

    const E6: u64 = 1_000_000;

    #[test]
    fn gaveta_igual_nao_anda() {
        let a = absorver_gaveta(500 * E6, 500 * E6, 1000 * E6, 7).unwrap();
        assert_eq!(a.delta_micro, 0);
        assert_eq!(a.indice_p, 7);
    }

    #[test]
    fn gaveta_maior_sobe_pelo_supply_do_instante() {
        // 2.000 USDC de P sobre 96.000 cotas: 2000/96000 por cota
        let a = absorver_gaveta(2000 * E6, 0, 96_000 * E6, 0).unwrap();
        assert_eq!(a.delta_micro, 2000 * E6);
        assert_eq!(
            a.indice_p,
            2000 * E6 as u128 * INDICE_SCALE / (96_000 * E6) as u128
        );
    }

    #[test]
    fn gaveta_menor_recusa_e_nunca_desconta() {
        let e = absorver_gaveta(499 * E6, 500 * E6, 1000 * E6, 7).unwrap_err();
        assert_eq!(e, DomError::GavetaDiminuiu.into());
    }

    #[test]
    fn supply_zero_guarda_o_delta_sem_indice() {
        let a = absorver_gaveta(10 * E6, 0, 0, 0).unwrap();
        assert_eq!(a.delta_micro, 10 * E6);
        assert_eq!(a.indice_p, 0);
    }

    #[test]
    fn entrada_ponderada_e_a_media_pelas_cotas() {
        // 100 cotas a 0 + 100 cotas a 10 → 5
        assert_eq!(entrada_ponderada(100, 0, 100, 10).unwrap(), 5);
        // primeiro lote: a entrada e' o indice de agora
        assert_eq!(entrada_ponderada(0, 0, 50, 42).unwrap(), 42);
        // lote novo de zero cotas: nada muda
        assert_eq!(entrada_ponderada(100, 3, 0, 99).unwrap(), 3);
    }

    #[test]
    fn aporte_com_indice_parado_nao_muda_o_ganho_de_quem_ja_estava() {
        // c1 cotas com entrada e1; indice I; aporte c2 ao proprio I.
        // ganho antes = c1 (I − e1); ganho depois = (c1+c2)(I − e') = c1 (I − e1)
        let (c1, e1, i, c2) = (55_000 * E6, 0u128, 3 * INDICE_SCALE / 100, 5_000 * E6);
        let antes = ganho(c1, e1, 0, i).unwrap();
        let e2 = entrada_ponderada(c1, e1, c2, i).unwrap();
        let depois = ganho(c1 + c2, e2, 0, i).unwrap();
        assert!(antes.abs_diff(depois) <= 1, "{antes} vs {depois}");
    }

    #[test]
    fn ganho_usa_a_entrada_efetiva_e_nunca_e_negativo() {
        let um = INDICE_SCALE;
        // entrou em 0, piso do ciclo em 1, referencia 3: conta de 1 a 3 → 2 por cota
        assert_eq!(ganho(10 * E6, 0, um, 3 * um).unwrap(), 20 * E6);
        // entrou em 2 (depois do piso 1): conta de 2 a 3
        assert_eq!(ganho(10 * E6, 2 * um, um, 3 * um).unwrap(), 10 * E6);
        // entrou depois da referencia: zero
        assert_eq!(ganho(10 * E6, 5 * um, um, 3 * um).unwrap(), 0);
    }

    /// A sequencia da mesa (D-F2-43 §2): 18 carteiras, 6 parcelas de P.
    /// Σ ganho = P ao micro (menos o truncamento por carteira), e a tabela da
    /// mesa bate ao centavo. O anti-teste ("fatia no fechamento × P depois da
    /// entrada") NAO conserva: da 3.886,85 de mesa em vez de 5.000.
    struct Sim {
        supply: u64,
        capital: u64,
        gaveta: u64,
        visto: u64,
        indice: u128,
        carteiras: Vec<(&'static str, u64, u128, usize)>,
        parcelas: Vec<u64>,
    }
    impl Sim {
        fn novo() -> Self {
            Sim {
                supply: 0,
                capital: 0,
                gaveta: 0,
                visto: 0,
                indice: 0,
                carteiras: vec![],
                parcelas: vec![],
            }
        }
        /// NAV de aporte = piso = (capital + gaveta) ÷ supply — sem marcacao na simulacao da mesa
        fn aporta(&mut self, nome: &'static str, usd: u64) {
            let nav = if self.supply == 0 {
                E6
            } else {
                ((self.capital as u128 + self.gaveta as u128) * E6 as u128 / self.supply as u128)
                    as u64
            };
            let cotas = (usd as u128 * E6 as u128 / nav as u128) as u64;
            self.carteiras
                .push((nome, cotas, self.indice, self.parcelas.len()));
            self.supply += cotas;
            self.capital += usd;
        }
        fn p(&mut self, valor: u64) {
            self.gaveta += valor;
            let a = absorver_gaveta(self.gaveta, self.visto, self.supply, self.indice).unwrap();
            self.visto = self.gaveta;
            self.indice = a.indice_p;
            self.parcelas.push(valor);
        }
    }

    #[test]
    fn a_simulacao_da_mesa_conserva_o_p_e_bate_a_tabela() {
        let mut s = Sim::novo();
        for (n, v) in [
            ("Egnon", 55_000),
            ("Filipe", 20_000),
            ("Leonardo", 12_000),
            ("Djan", 9_000),
        ] {
            s.aporta(n, v * E6);
        }
        s.p(2_000 * E6);
        for (n, v) in [
            ("Ariel", 8_000),
            ("Gisleno", 6_500),
            ("Silvio", 5_000),
            ("Rosane", 500),
        ] {
            s.aporta(n, v * E6);
        }
        s.p(1_500 * E6);
        for (n, v) in [("Raony", 1_500), ("Abraao", 800), ("Hugo", 300)] {
            s.aporta(n, v * E6);
        }
        s.p(2_500 * E6);
        for (n, v) in [("Edson", 15_000), ("Ismar", 7_000), ("Rafael", 4_000)] {
            s.aporta(n, v * E6);
        }
        s.p(1_800 * E6);
        for (n, v) in [("Victor", 2_500), ("Marcos", 10_000)] {
            s.aporta(n, v * E6);
        }
        s.p(1_200 * E6);
        for (n, v) in [("Tatiane", 3_000), ("Bruno", 6_000)] {
            s.aporta(n, v * E6);
        }
        s.p(1_000 * E6);
        assert_eq!(s.gaveta, 10_000 * E6);

        let mut soma: u64 = 0;
        let mut por_nome = std::collections::HashMap::new();
        for (nome, cotas, entrada, _) in &s.carteiras {
            let g = ganho(*cotas, *entrada, 0, s.indice).unwrap();
            soma += g;
            por_nome.insert(*nome, g);
        }
        assert!(s.gaveta - soma < 18, "Σ ganho {soma} vs P {}", s.gaveta);
        let perto = |a: u64, b: u64| a.abs_diff(b) <= 10_000; // um centavo
        assert!(
            perto(por_nome["Egnon"], 4_482_330_000),
            "Egnon {}",
            por_nome["Egnon"]
        );
        assert!(
            perto(por_nome["Filipe"], 1_629_940_000),
            "Filipe {}",
            por_nome["Filipe"]
        );
        assert!(
            perto(por_nome["Ariel"], 475_400_000),
            "Ariel {}",
            por_nome["Ariel"]
        );
        assert!(
            perto(por_nome["Rosane"], 29_710_000),
            "Rosane {}",
            por_nome["Rosane"]
        );
        assert!(
            perto(por_nome["Edson"], 377_060_000),
            "Edson {}",
            por_nome["Edson"]
        );
        assert!(
            perto(por_nome["Bruno"], 34_270_000),
            "Bruno {}",
            por_nome["Bruno"]
        );

        // ANTI-TESTE: fatia no fechamento × Σ parcelas depois da entrada — NAO e' a regra, e nao conserva
        let mut anti_mesa: u128 = 0;
        for (_, cotas, _, desde) in &s.carteiras {
            let p_depois: u64 = s.parcelas[*desde..].iter().sum();
            anti_mesa += *cotas as u128 * p_depois as u128 / s.supply as u128 / 2;
        }
        assert!(
            anti_mesa.abs_diff(3_886_850_000) < 20_000,
            "anti-teste {anti_mesa}"
        );
        assert_ne!(anti_mesa, 5_000 * E6 as u128);
    }
}

/// A PDA que e' dona da gaveta, e a gaveta (ATA de USDC dela, no programa de token dado).
/// Aplica uma leitura da gaveta ao cofre: confere que a conta é A gaveta,
/// absorve o delta (índice + `p_ciclo` + saldo visto) e devolve quanto entrou.
///
/// É o único lugar que escreve `indice_p`, `p_ciclo` e `gaveta_saldo_visto`
/// fora do fechamento. `deposit`, `deposit_para` e `publish_nav` chamam isto
/// ANTES de qualquer conta que dependa do índice.
pub fn absorver_no_cofre(
    vault: &mut crate::state::Vault,
    gaveta_key: &Pubkey,
    gaveta_saldo: u64,
    supply_raw: u64,
) -> Result<u64> {
    require_keys_eq!(*gaveta_key, vault.gaveta_usdc, DomError::GavetaErrada);
    require!(
        vault.gaveta_usdc != Pubkey::default(),
        DomError::GavetaErrada
    );
    let a = absorver_gaveta(
        gaveta_saldo,
        vault.gaveta_saldo_visto,
        supply_raw,
        vault.indice_p,
    )?;
    vault.indice_p = a.indice_p;
    vault.gaveta_saldo_visto = gaveta_saldo;
    vault.p_ciclo = vault
        .p_ciclo
        .checked_add(a.delta_micro)
        .ok_or(DomError::MathOverflow)?;
    Ok(a.delta_micro)
}

/// Registra um lote de cotas que entrou numa carteira: cria/atualiza
/// `PosicaoDoCotista` pela média ponderada. `cotas_antes` é o saldo da ATA
/// ANTES do lote (o handler lê antes do mint/transfer).
pub fn registrar_entrada(
    posicao: &mut crate::state::PosicaoDoCotista,
    owner: &Pubkey,
    bump: u8,
    cotas_antes: u64,
    cotas_novas: u64,
    indice_agora: u128,
) -> Result<()> {
    if posicao.owner == Pubkey::default() {
        posicao.owner = *owner;
        posicao.bump = bump;
        posicao.indice_entrada = 0;
    }
    require_keys_eq!(posicao.owner, *owner, DomError::PosicaoDoCotistaAusente);
    posicao.indice_entrada = entrada_ponderada(
        cotas_antes,
        posicao.indice_entrada,
        cotas_novas,
        indice_agora,
    )?;
    Ok(())
}

// ─── J7 — o acerto preguiçoso por carteira ───────────────────────────────────

/// O que uma carteira deve e o que já pagou desde o último acerto.
#[derive(Debug, PartialEq, Eq)]
pub struct Acerto {
    /// taxa_mesa × ganho nos ciclos JA FECHADOS desde o último acerto (micro-USDC).
    pub devido_micro: u64,
    /// cotas × (indice_diluicao − indice_diluicao_visto): o que a diluição já tirou dela (micro-USDC).
    pub pago_micro: u64,
}

/// A conta do acerto. `devido` só conta ciclos fechados (`indice_ciclo`), nunca o
/// P ainda aberto; `pago` conta toda diluição que aconteceu desde o último acerto.
pub fn acerto_de(
    cotas_raw: u64,
    taxa_mesa_bps: u16,
    posicao: &crate::state::PosicaoDoCotista,
    vault: &crate::state::Vault,
) -> Result<Acerto> {
    let ganho_fechado = ganho(
        cotas_raw,
        posicao.indice_entrada.max(posicao.indice_p_acertado),
        0,
        vault.indice_ciclo,
    )?;
    let devido_micro = u64::try_from((ganho_fechado as u128) * (taxa_mesa_bps as u128) / 10_000)
        .map_err(|_| error!(DomError::MathOverflow))?;
    let pago_micro = if vault.indice_diluicao > posicao.indice_diluicao_visto {
        u64::try_from(
            (cotas_raw as u128) * (vault.indice_diluicao - posicao.indice_diluicao_visto)
                / INDICE_SCALE,
        )
        .map_err(|_| error!(DomError::MathOverflow))?
    } else {
        0
    };
    Ok(Acerto {
        devido_micro,
        pago_micro,
    })
}

/// Uma carteira está acertada quando os dois odômetros batem com o cofre.
pub fn acertada(posicao: &crate::state::PosicaoDoCotista, vault: &crate::state::Vault) -> bool {
    posicao.indice_diluicao_visto == vault.indice_diluicao
        && posicao.indice_p_acertado == vault.indice_ciclo
}

/// **No J toda cota vive na ATA do dono.** O acerto (J7) e o ganho (J4) são
/// por CARTEIRA (a posição é uma por dono), mas o saldo é lido de UMA conta de
/// token. Se o dono pudesse guardar cota numa segunda conta, acertaria pela
/// menor, marcaria a posição como em dia e a maior escaparia da taxa. Então
/// toda porta que lê cota para o índice — e o hook, na entrada — exige a ATA
/// canônica (`[owner, Token-2022, mint]`). Em mainnet (21/09) as 20 contas de
/// cota são ATAs; a única exceção é o escrow do resgate, PDA do programa,
/// isento no hook.
pub fn exigir_ata(
    conta: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Result<()> {
    let esperada = anchor_spl::associated_token::get_associated_token_address_with_program_id(
        owner,
        mint,
        token_program,
    );
    require_keys_eq!(*conta, esperada, DomError::ContaDeCotaNaoEAta);
    Ok(())
}

/// O que o hook pode fazer sem CPI: aceitar a carteira acertada, ou a que
/// **nada deve e nada tem a receber** (devido == pago — uma carteira sozinha
/// no cofre, ou sem cota) e, nesse caso, andar os odômetros. Qualquer saldo
/// líquido é `AcertoPendente`: o dono assina `acertar` antes.
pub fn exigir_acertada(
    cotas_raw: u64,
    posicao: &mut crate::state::PosicaoDoCotista,
    vault: &crate::state::Vault,
) -> Result<()> {
    if acertada(posicao, vault) {
        return Ok(());
    }
    let a = acerto_de(cotas_raw, vault.perf_fee_bps_total, posicao, vault)?;
    require!(a.devido_micro == a.pago_micro, DomError::AcertoPendente);
    posicao.indice_diluicao_visto = vault.indice_diluicao;
    posicao.indice_p_acertado = vault.indice_ciclo;
    Ok(())
}

#[cfg(test)]
mod provas_do_acerto {
    use super::*;
    use crate::state::{PosicaoDoCotista, Vault};

    fn vault(indice_ciclo: u128, indice_diluicao: u128) -> Vault {
        let mut v: Vault = unsafe { std::mem::zeroed() };
        v.indice_ciclo = indice_ciclo;
        v.indice_diluicao = indice_diluicao;
        v
    }
    fn pos(entrada: u128, visto: u128, acertado: u128) -> PosicaoDoCotista {
        PosicaoDoCotista {
            owner: Pubkey::default(),
            indice_entrada: entrada,
            indice_diluicao_visto: visto,
            indice_p_acertado: acertado,
            bump: 0,
        }
    }
    const E6: u64 = 1_000_000;

    #[test]
    fn devido_e_metade_do_ganho_fechado_e_pago_e_a_diluicao_vista() {
        let um = INDICE_SCALE;
        // ganho fechado: 10 cotas × (indice_ciclo 3 − entrada 1) = 20 USDC → devido 10 (50%)
        // diluicao: 10 cotas × (0,8 − 0,3) = 5 USDC pagos
        let a = acerto_de(
            10 * E6,
            5000,
            &pos(um, 3 * um / 10, 0),
            &vault(3 * um, 8 * um / 10),
        )
        .unwrap();
        assert_eq!(
            a,
            Acerto {
                devido_micro: 10 * E6,
                pago_micro: 5 * E6
            }
        );
    }

    #[test]
    fn o_p_do_ciclo_aberto_nao_conta_no_devido() {
        // indice_p pode estar alem de indice_ciclo; o devido para em indice_ciclo
        let um = INDICE_SCALE;
        let a = acerto_de(10 * E6, 5000, &pos(0, 0, 0), &vault(2 * um, 0)).unwrap();
        assert_eq!(a.devido_micro, 10 * E6); // 10 × 2 × 50%
    }

    #[test]
    fn acerto_e_idempotente() {
        let um = INDICE_SCALE;
        let v = vault(3 * um, um);
        let p = pos(um, um, 3 * um); // ja' acertada ate' o ciclo 3 e a diluicao 1
        let a = acerto_de(10 * E6, 5000, &p, &v).unwrap();
        assert_eq!(
            a,
            Acerto {
                devido_micro: 0,
                pago_micro: 0
            }
        );
        assert!(acertada(&p, &v));
    }

    #[test]
    fn exigir_acertada_aceita_so_o_liquido_zero_e_anda_os_odometros() {
        let um = INDICE_SCALE;
        // taxa 50%: 10 cotas × (ciclo 2 − entrada 0) = 20 → devido 10; diluicao 1,0 × 10 cotas = 10 pagos → liquido zero
        let mut v = vault(2 * um, um);
        v.perf_fee_bps_total = 5000;
        let mut p = pos(0, 0, 0);
        exigir_acertada(10 * E6, &mut p, &v).unwrap();
        assert!(acertada(&p, &v), "odometros andaram");
        // com diluicao menor, deve 10 e pagou 5: pendente
        let v2 = {
            let mut v2 = vault(2 * um, um / 2);
            v2.perf_fee_bps_total = 5000;
            v2
        };
        let mut p2 = pos(0, 0, 0);
        assert!(exigir_acertada(10 * E6, &mut p2, &v2).is_err());
        assert!(!acertada(&p2, &v2));
        // sem cota, nunca pendente
        let mut p3 = pos(0, 0, 0);
        exigir_acertada(0, &mut p3, &v2).unwrap();
        assert!(acertada(&p3, &v2));
    }

    #[test]
    fn quem_entrou_depois_do_fechamento_nao_deve_nada_mas_pagou_diluicao() {
        let um = INDICE_SCALE;
        // entrou com indice 5 (depois do indice_ciclo 3): devido 0; mas a diluicao andou 0,1 → pagou 1 → credito
        let a = acerto_de(10 * E6, 5000, &pos(5 * um, 0, 0), &vault(3 * um, um / 10)).unwrap();
        assert_eq!(
            a,
            Acerto {
                devido_micro: 0,
                pago_micro: E6
            }
        );
    }
}

/// O acerto por CPI, compartilhado: `deposit` (dono assina), `deposit_para`
/// (nao assina: so' credito), `acertar`, e as saidas. `posicao` recem-criada
/// (owner zerado) nao tem historico: nasce acertada nos dois odometros.
pub fn acertar_com_cpi<'info>(
    vault: &mut Account<'info, crate::state::Vault>,
    posicao: &mut crate::state::PosicaoDoCotista,
    owner_dom: &InterfaceAccount<'info, anchor_spl::token_interface::TokenAccount>,
    dom_mint: &InterfaceAccount<'info, anchor_spl::token_interface::Mint>,
    owner: &AccountInfo<'info>,
    owner_assina: bool,
    dom_token_program: &Interface<'info, anchor_spl::token_interface::TokenInterface>,
) -> Result<()> {
    use anchor_spl::token_interface::{burn, mint_to, Burn, MintTo};
    exigir_ata(
        &owner_dom.key(),
        owner.key,
        &dom_mint.key(),
        dom_token_program.key,
    )?;
    if posicao.owner == Pubkey::default() {
        posicao.indice_diluicao_visto = vault.indice_diluicao;
        posicao.indice_p_acertado = vault.indice_ciclo;
        return Ok(());
    }
    if acertada(posicao, vault) {
        return Ok(());
    }
    let cotas = owner_dom.amount;
    let a = acerto_de(cotas, vault.perf_fee_bps_total, posicao, vault)?;
    let nav = if vault.nav_fechamento > 0 {
        vault.nav_fechamento
    } else {
        vault.nav
    };
    if a.pago_micro > a.devido_micro {
        let credito = crate::math::shares_from_usdc(a.pago_micro - a.devido_micro, nav)?;
        if credito > 0 {
            let bump = [vault.bump];
            let seeds: &[&[&[u8]]] = &[&[crate::constants::VAULT_SEED, &bump]];
            mint_to(
                CpiContext::new_with_signer(
                    dom_token_program.key(),
                    MintTo {
                        mint: dom_mint.to_account_info(),
                        to: owner_dom.to_account_info(),
                        authority: vault.to_account_info(),
                    },
                    seeds,
                ),
                credito,
            )?;
        }
    } else if a.devido_micro > a.pago_micro {
        require!(owner_assina && owner.is_signer, DomError::AcertoPendente);
        let debito = crate::math::shares_from_usdc(a.devido_micro - a.pago_micro, nav)?.min(cotas);
        if debito > 0 {
            burn(
                CpiContext::new(
                    dom_token_program.key(),
                    Burn {
                        mint: dom_mint.to_account_info(),
                        from: owner_dom.to_account_info(),
                        authority: owner.clone(),
                    },
                ),
                debito,
            )?;
        }
    }
    posicao.indice_diluicao_visto = vault.indice_diluicao;
    posicao.indice_p_acertado = vault.indice_ciclo;
    Ok(())
}
