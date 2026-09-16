# DOM — espelho do build

**Este repositorio nao e' editado. Ele e' GERADO** por `scripts/espelho.sh`, no
repositorio de trabalho, a cada upgrade. Pull request aqui nao alcanca o programa.

Existe para uma coisa so': **qualquer pessoa refazer o binario e conferir o
`sha256` contra o que esta' na rede.**

> ⚠️ **Este espelho NAO e' o binario instalado.** Ele reproduz uma
> versao construida e ainda nao publicada.
>
> instalado em mainnet ... `a877184562c7d9967f226c560182e86d827394847367b424a6891cd5e1b597be`
> este espelho .......... `1038f54bde8a416f08790bafceda6b835444161c96096a877e084b7446c3e9ff`
>
> Conferir contra a rede hoje vai dar diferenca, e a diferenca e' esta.


## Upgrade I — crédito

A correção que este binário carrega em `publish_nav` — o intervalo mínimo do
oráculo passando a valer em **tempo real**, com `MAX_NAV_TIMESTAMP_LAG` e o erro
`6077 NavTimestampMuitoAntigo` — foi reportada e escrita por **Filipe Brito**
(`filipemb`), pelo canal do `security.txt` compilado no binário, como o
[PR #1](https://github.com/cfsumbach/dom-vault/pull/1) deste espelho, em
2026-09-16. O patch entrou como ele o escreveu. Este repositório é gerado e não
recebe merge direto — por isso o PR fica aberto como registro, e a correção
chega por aqui.

## O que este espelho reproduz

| | |
|---|---|
| programa | `2KRqqGA47Pg2ML7Q8WJyaVAmEL8DaRx1sKxqzpVyNnkg` |
| `sha256` do `.so` | `1038f54bde8a416f08790bafceda6b835444161c96096a877e084b7446c3e9ff` |
| tamanho | 693392 bytes |
| instrucoes no IDL | 26 |
| erros no IDL | 78 |
| gerado em | 2026-09-16T14:59:42Z |

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

### Contra a rede — e o tamanho NAO e' detalhe

```bash
solana program dump 2KRqqGA47Pg2ML7Q8WJyaVAmEL8DaRx1sKxqzpVyNnkg rede.so --url mainnet-beta
head -c 693392 rede.so | sha256sum       # 693392 = o tamanho da tabela acima
```

⚠️ **`sha256sum rede.so` direto NAO bate, e nao e' sinal de adulteracao.**

A `programdata` e' maior que o programa e o resto vem zerado. O `dump` traz a
conta inteira, entao o arquivo tem o binario mais o padding — e quanto mais a
conta foi estendida, maior o padding.

⚠️ **E "tirar os zeros do fim" TAMBEM nao bate.** O proprio binario termina em
zeros, entao remover todos come alguns bytes do programa e muda o hash. Ja'
custou uma conferencia em 14/09, com esta instrucao no lugar.

**O unico caminho que fecha e' o tamanho exato**, que e' por isso que ele esta'
na tabela acima e nao so' o `sha256`.

### A toolchain que reproduz

| | |
|---|---|
| rust | `rustc 1.89.0 (29483883e 2025-08-04)` — prendido em `rust-toolchain.toml` |
| solana / agave | `solana-cli 3.1.10 (src:7bc9c805; feat:1620780344, client:Agave)` |
| anchor | `anchor-cli 1.1.2` |

O `cargo build-sbf` vem do `solana-cli`: e' ele quem traz o LLVM do SBF, e e' a
versao dele que decide o resultado. **Versao diferente pode dar hash diferente
com o mesmo fonte** — isso nao e' adulteracao, e' toolchain.

## O que NAO esta aqui

Procedimentos de operacao, runbook de cerimonia, evidencias, relatorios e o
registro de decisoes ficam no repositorio de trabalho, privado. **Nao foram
filtrados: o build nao precisa deles.**
