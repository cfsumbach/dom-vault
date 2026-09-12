# DOM — espelho do build

**Este repositorio nao e' editado. Ele e' GERADO** por `scripts/espelho.sh`, no
repositorio de trabalho, a cada upgrade. Pull request aqui nao alcanca o programa.

Existe para uma coisa so': **qualquer pessoa refazer o binario e conferir o
`sha256` contra o que esta' na rede.**

> ✅ **Este espelho reproduz o binario INSTALADO EM MAINNET.**
> `9215d35797dc2c6f72542c64c9e9a2c553bf54bbf67f6e92277f8e48d7416ae5`

## O que este espelho reproduz

| | |
|---|---|
| programa | `2KRqqGA47Pg2ML7Q8WJyaVAmEL8DaRx1sKxqzpVyNnkg` |
| `sha256` do `.so` | `9215d35797dc2c6f72542c64c9e9a2c553bf54bbf67f6e92277f8e48d7416ae5` |
| tamanho | 665872 bytes |
| instrucoes no IDL | 25 |
| erros no IDL | 77 |
| gerado em | 2026-09-12T21:48:13Z |

## Sobre o `idl/dom_vault.json` — leia antes de vendorizar

O nome **nao carrega a versao do layout**, e isso e' armadilha conhecida: nome sem
versao sugere "o canonico", e cliente que o trate assim le' conta de outro layout
**sem estourar** e mostra numero trocado. Falha muda e' pior que falha alta.

Ele fica assim porque o `tests/config.rs` o inclui em tempo de compilacao, e
renomear quebraria o build.

**Quem vendorizar: renomeie do seu lado com a versao no nome, e escolha o molde
pela CONTA — pelo tamanho dela e pelo `layout_version` que ela carrega —, nunca
por configuracao.** A conta e' a verdade; a configuracao e' o que envelhece.

O `Vault` deste IDL tem campo `layout_version`, e
ele e' o **ultimo** campo de proposito: le-se em `data[len-2..len]` sem conhecer o
resto do layout.

## Os parametros de politica NAO se leem daqui

Onze valores — pisos de entrada e saida, prazos, cap, reserva, taxa — sao **campo
da conta**, ajustaveis por proposta do multisig. As `#[constant]` que aparecem
neste IDL para eles sao o **valor de partida**, nao o vigente.

Ler do IDL compila, nao da' erro, e mostra valor errado no dia seguinte a' primeira
votacao. **Leia da conta.**

## Como conferir

```bash
cargo build-sbf
sha256sum target/deploy/dom_vault.so    # tem de dar o sha da tabela acima
```

Contra a rede:

```bash
solana program dump 2KRqqGA47Pg2ML7Q8WJyaVAmEL8DaRx1sKxqzpVyNnkg rede.so --url mainnet-beta
```

A `programdata` pode ter zero-padding depois do fim do programa — compare o
**prefixo** do tamanho do `.so`, e confira que a cauda e' toda zero.

## O que NAO esta aqui

Procedimentos de operacao, runbook de cerimonia, evidencias, relatorios e o
registro de decisoes ficam no repositorio de trabalho, privado. **Nao foram
filtrados: o build nao precisa deles.**
