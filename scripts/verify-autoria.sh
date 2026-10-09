#!/usr/bin/env bash
# =============================================================================
# $DOM — Verificação de autoria. Evidência reproduzível, não check de uma vez só.
# =============================================================================
# Uso:  bash scripts/verify-autoria.sh
# Saída limpa + exit 0 = conforme. Qualquer hit = exit 1.
#
# Entra no checklist de publicação do repo (pré verifiable build) e nas
# evidências de fechamento de fase.
# =============================================================================
set -uo pipefail
cd "$(dirname "$0")/.."

AUTOR_ESPERADO="Carlos F Sumbach <sumbach@pm.me>"
EMAIL_ESPERADO="sumbach@pm.me"

# ---------------------------------------------------------------------------
# ⚠️ O E-MAIL HISTORICO DESTE REPO — DECISOES 09/10 18:25
# ---------------------------------------------------------------------------
# Seis dos sete commits deste espelho foram assinados com outro e-mail do dono,
# do dominio da empresa. A decisao e explicita: "e-mail canonico sumbach@pm.me;
# historico com o outro FICA".
#
# Entao ele e ACEITO para o passado e **DECLARADO** na saida, em amarelo, como as
# excecoes nominais de commit. Nao e silenciado: e-mail aceito que desaparece da
# tela e e-mail que vira regra.
#
# ⚠️ E ele NAO vale depois do corte. Commit novo com o e-mail da empresa reprova.
# ---------------------------------------------------------------------------
EMAIL_HISTORICO="carlos@depthofmarket.com.br"

# ---------------------------------------------------------------------------
# O CORTE — DECISOES 09/10 18:00
# ---------------------------------------------------------------------------
# O nome do autor mudou em 09/10/2026, por decisão do dono: todo commit novo sai
# como "Carlos F Sumbach <sumbach@pm.me>". Antes do corte são 312 commits como
# "sumbach <sumbach@pm.me>" — o MESMO e-mail, outro nome.
#
# ⚠️ E ISSO QUEBROU ESTE PORTÃO no dia da mudança. Ele exigia que TODOS os
# autores de TODA a história fossem idênticos a uma única string. Com dois nomes
# no log e reescrita de história proibida, **nenhum valor de `AUTOR_ESPERADO`
# passava** — nem o antigo, nem o novo.
#
# A decisão resolve mudando a PERGUNTA, não o valor: valida-se **só a faixa de
# commits novos**, e do passado cobra-se apenas o e-mail.
#
# A faixa sai do REMOTO, nunca de um sha escrito à mão — sha à mão envelhece em
# silêncio, como aconteceu com o carimbo do IDL.
#
# ⚠️ E O CORTE É POR DATA, não pela faixa, e isso FOI UMA LACUNA DA DECISÃO.
# Um ramo de tarefa aberto antes das 18:00 tem, DENTRO da faixa, commits
# assinados com o nome antigo — legítimos, porque o nome era aquele quando foram
# escritos. A própria decisão fala de "histórico anterior ao CORTE", então o
# corte tem data e a faixa é só o escopo. Sem isto, todo ramo vivo em 09/10
# ficaria vermelho para sempre — e vermelho permanente é vermelho que ninguém lê.
# ---------------------------------------------------------------------------
CORTE="2026-10-09T18:00:00"

if git rev-parse --verify --quiet origin/dev >/dev/null; then
  BASE="origin/dev"
elif git rev-parse --verify --quiet origin/main >/dev/null; then
  BASE="origin/main"
else
  BASE=""
fi
if [ -n "$BASE" ]; then FAIXA="$BASE..HEAD"; else FAIXA="HEAD"; fi

# ---------------------------------------------------------------------------
# ISENÇÃO: os IDLs carimbados — DECISOES 09/10 18:00
# ---------------------------------------------------------------------------
# `idl/dom_vault.*.json` são a PROVA de qual binário está no ar: o sha de cada um
# está travado em `backend-nav/lib/planta.ts`. Um byte de doc comment mudado ali
# quebra a verificação de planta, que é o que impede o backend de decodificar o
# cofre com offsets de outra era.
#
# ⚠️ Eles carregam termo de nomenclatura porque são GERADOS do comentário Rust. A
# limpeza correta é no comentário de origem + rebuild, e o rebuild muda o sha —
# então os carimbados ficam como estão, e a varredura os isenta.
# ---------------------------------------------------------------------------
# ⚠️ AQUI O IDL SE CHAMA `dom_vault.json`, sem letra de planta — e o padrao dos
# outros repos (`dom_vault.*.json`) NAO casaria com ele, porque exige um ponto
# depois de `dom_vault.`. Conferido antes de escrever: sem esta linha, a isencao
# da decisao de 18:25 nao pegaria o unico IDL que este repo tem.
ISENTOS=':!idl/dom_vault*.json'

# O padrão é montado por concatenação de fragmentos de propósito: escrito
# inteiro, este script casaria consigo mesmo e a verificação nunca passaria.
PAT="$(printf '%s|%s|%s|%s|%s|%s' \
  'cla''ude' 'anthro''pic' 'co-''authored' 'gener''ated with' 'CC''ODE' 'CC-DOM''-FUNDO')"

# ---------------------------------------------------------------------------
# EXCEÇÕES NOMINAIS — D-F2-29
# ---------------------------------------------------------------------------
# DOIS commits de 2026-09-09 carregam rodapé de agente na mensagem. A regra
# falhou na origem, e está consertada: todo commit posterior sai limpo.
#
# Não foram reescritos. Reescrever `main` compartilhada com force-push é uma
# ferramenta destrutiva — a que quase apagou trabalho de outra frente em
# 2026-09-03 — e aqui ela seria usada contra duas linhas de rodapé. O remédio
# custaria mais que a doença.
#
# **Ficam DECLARADOS, nunca silenciados.** Este script os imprime em toda
# execução, em amarelo. Exceção que some da tela é exceção que vira regra.
#
# Não acrescente hash a esta lista sem uma decisão escrita. A lista é o registro
# de uma falha, não uma válvula de escape.
# ---------------------------------------------------------------------------
# ⚠️ VAZIO, E ISSO FOI MEDIDO: `git log --all -i --grep` dos seis termos neste
# repo devolve ZERO. O espelho nasceu limpo — nao ha falha a registrar aqui, e
# por isso a lista nao tem hash nenhum.
EXCECOES_DECLARADAS=""

FALHOU=0

hr() { printf '%s\n' "=============================================="; }
falha() { printf '  \033[1;31mFALHOU\033[0m %s\n' "$1"; FALHOU=1; }
passa() { printf '  \033[1;32mOK\033[0m %s\n' "$1"; }

hr; echo " 1. AUTORIA DOS COMMITS"; hr
echo "  faixa nova ....... $FAIXA"
N_FAIXA=$(git log --format='%H' "$FAIXA" 2>/dev/null | wc -l | tr -d ' ')
N_POS=$(git log --format='%H' --since="$CORTE" "$FAIXA" 2>/dev/null | wc -l | tr -d ' ')
echo "  commits na faixa . $N_FAIXA  (depois do corte $CORTE: $N_POS)"
echo

# --- 1a · DEPOIS DO CORTE: nome E e-mail exatos, autor E committer -----------
if [ "$N_POS" -eq 0 ]; then
  passa "nenhum commit novo depois do corte: nada a cobrar pelo nome"
else
  git log --format="  %h  %an <%ae>  |  %s" --since="$CORTE" "$FAIXA" | cut -c1-110
  echo
  A_FAIXA=$(git log --format="%an <%ae>" --since="$CORTE" "$FAIXA" | sort -u)
  C_FAIXA=$(git log --format="%cn <%ce>" --since="$CORTE" "$FAIXA" | sort -u)
  if [ "$A_FAIXA" = "$AUTOR_ESPERADO" ]; then
    passa "autores depois do corte: $AUTOR_ESPERADO"
  else
    falha "autor divergente DEPOIS DO CORTE:"; echo "$A_FAIXA" | sed 's/^/      /'
  fi
  if [ "$C_FAIXA" = "$AUTOR_ESPERADO" ]; then
    passa "committers depois do corte: $AUTOR_ESPERADO"
  else
    falha "committer divergente DEPOIS DO CORTE:"; echo "$C_FAIXA" | sed 's/^/      /'
  fi
fi

# --- 1b · O PASSADO: so' o e-mail --------------------------------------------
# Nome antigo e aceito; e-mail de terceiro, nunca. O que esta regra protege e a
# identidade de quem assina, nao a grafia do nome.
FORA=$(git log --format="%ae%n%ce" | sort -u \
       | grep -v "^${EMAIL_ESPERADO}$" | grep -v "^${EMAIL_HISTORICO}$" || true)
if [ -z "$FORA" ]; then
  passa "historico nos e-mails do dono ($(git log --oneline | wc -l | tr -d ' ') commits)"
else
  falha "e-mail de TERCEIRO no historico:"; printf '%s\n' "$FORA" | sed 's/^/      /'
fi
# O e-mail historico aparece SEMPRE, e nunca como OK.
if git log --format='%ae%n%ce' | grep -q "^${EMAIL_HISTORICO}$"; then
  N_HIST=$(git log --format='%ae' | grep -c "^${EMAIL_HISTORICO}$")
  printf '  \033[1;33mEXCECAO\033[0m e-mail historico %s em %s commit(s)  (DECISOES 18:25 — fica)\n' \
    "$EMAIL_HISTORICO" "$N_HIST"
fi
# E depois do CORTE ele nao vale.
if [ "$N_POS" -gt 0 ] && git log --format='%ae%n%ce' --since="$CORTE" "$FAIXA" | grep -q "^${EMAIL_HISTORICO}$"; then
  falha "e-mail historico usado DEPOIS do corte: so o canonico vale para commit novo"
fi

echo; hr; echo " 2. WORKING TREE (so' o que o git rastreia)"; hr
# ---------------------------------------------------------------------------
# ⚠️ `git grep` E NAO `grep -r`, desde 09/10 — e a troca tem motivo.
#
# `grep -r` varria TUDO em disco, inclusive arquivo LOCAL que o `.git/info/exclude`
# mantem fora do versionamento DE PROPOSITO (DECISOES 17:13). O portao ficava
# vermelho por causa de arquivo que, por decisao do dono, nunca entra no repo —
# cobrando do DISCO uma regra que e do REPOSITORIO.
#
# `git grep` olha so' o rastreado. O que nao viaja num push nao e conteudo do
# repo: a mesma logica que ja isentava o `info/exclude` na secao 4.
# ---------------------------------------------------------------------------
if git grep -nIiE -e "$PAT" -- . "$ISENTOS" 2>/dev/null | grep -q .; then
  git grep -nIiE -e "$PAT" -- . "$ISENTOS" 2>/dev/null | head -20
  falha "ocorrências em arquivo RASTREADO (acima)"
else
  passa "arquivos rastreados limpos (locais fora do git nao sao cobrados)"
fi

echo; hr; echo " 3. MENSAGENS DE COMMIT"; hr
# Sem -r: com -r e sem path, o GNU grep recursa no diretório em vez de ler
# o pipe, e esta seção passaria a re-varrer a árvore sem olhar as mensagens.
# ⚠️ So' a FAIXA NOVA (DECISOES 18:00), e cobrando MENSAGEM **e** ARQUIVOS que o
# commit toca — nome e conteudo. O passado tem o `a712be4` declarado abaixo.
SUJOS=""
while read -r H; do
  [ -n "$H" ] || continue
  case " $EXCECOES_DECLARADAS " in *" $H "*) continue ;; esac
  if git log -1 --format="%B" "$H" | grep -iqE "$PAT"; then
    printf '  %s  (mensagem)\n' "$(git log -1 --format='%h %s' "$H")"
    SUJOS="$SUJOS $H"; continue
  fi
  TOCADOS="$(git show --name-only --format='' "$H" -- . "$ISENTOS" 2>/dev/null | grep -v '^$' || true)"
  [ -n "$TOCADOS" ] || continue
  if printf '%s\n' "$TOCADOS" | grep -iqE "$PAT"; then
    printf '  %s  (NOME de arquivo tocado)\n' "$(git log -1 --format='%h %s' "$H")"
    SUJOS="$SUJOS $H"; continue
  fi
  if printf '%s\n' "$TOCADOS" | xargs -r -I{} git grep -lIiE -e "$PAT" "$H" -- {} 2>/dev/null | grep -q .; then
    printf '  %s  (CONTEUDO de arquivo tocado)\n' "$(git log -1 --format='%h %s' "$H")"
    SUJOS="$SUJOS $H"
  fi
done < <(git log --format="%H" "$FAIXA" 2>/dev/null)

if [ -n "$SUJOS" ]; then
  falha "ocorrências na FAIXA NOVA (acima)"
else
  passa "faixa nova limpa: mensagem, nome e conteudo dos arquivos tocados"
fi

# As exceções aparecem SEMPRE, e nunca como OK. Ver D-F2-29.
for E in $EXCECOES_DECLARADAS; do
  git rev-parse --verify --quiet "$E" >/dev/null || continue
  printf '  \033[1;33mEXCECAO\033[0m %s  (D-F2-29 — nao reescrito de proposito)\n' \
    "$(git log -1 --format='%h %s' "$E")"
done

echo; hr; echo " 4. INTERNALS DO .git"; hr
if [ -d .git/refs/original ]; then
  falha "refs/original ainda existe (reescrita não foi purgada)"
else
  passa "refs/original removido"
fi
# ---------------------------------------------------------------------------
# `.git/info/exclude` fica FORA desta varredura, e apenas ele.
#
# O vermelho de 25/09: o portão acusou o arquivo de exclusão local porque ele
# ganhou linhas apontando a pasta oculta da ferramenta que roda o agente —
# escritas por ela mesma, para que a tralha dela não apareça no `git status`.
# São PADRÕES DE IGNORAR, locais, que nunca viajam num push: o arquivo não é
# conteúdo do repositório nem história dele.
#
# Tirar as linhas devolveria a tralha ao `git status` e um `add -A` distraído a
# commitaria — o remédio criaria a doença que o portão existe para impedir.
#
# Mas ele não sai de graça: o que se cobra do arquivo é que tenha SÓ padrões
# (linhas de caminho e comentários). Rodapé de agente, nome de autor ou
# mensagem escondida ali continuam vermelhos.
#
# ⚠️ E DESDE 09/10 saem tambem o `COMMIT_EDITMSG` e os indices de worktree, pela
# MESMA razao: sao rascunho LOCAL que o git reescreve a cada commit e que nunca
# viaja num push. O `COMMIT_EDITMSG` guarda a ULTIMA mensagem digitada — se ela
# citou um termo para EXPLICAR a regra, o portao acusava o rascunho de uma
# mensagem que nem foi commitada. Cobrar isso e cobrar o lapis, nao o texto.
#
# O que continua vermelho dentro de `.git`: qualquer OUTRO arquivo — objeto,
# pacote, ref, hook, config.
EXCLUDE=".git/info/exclude"
LOCAIS="^\\./\\.git/(info/exclude|COMMIT_EDITMSG|MERGE_MSG|SQUASH_MSG|worktrees/.*)$"
if grep -riEl "$PAT" .git 2>/dev/null | sed 's|^|./|' | grep -vE "$LOCAIS" | grep -q .; then
  grep -riEl "$PAT" .git 2>/dev/null | sed 's|^|./|' | grep -vE "$LOCAIS"
  falha "resquício dentro de .git (arquivos acima)"
elif [ -f "$EXCLUDE" ] && grep -vE '^[[:space:]]*(#|$)' "$EXCLUDE" | grep -qvE '^[!/*a-zA-Z0-9._-][^[:space:]]*$'; then
  grep -vE '^[[:space:]]*(#|$)' "$EXCLUDE" | grep -vE '^[!/*a-zA-Z0-9._-][^[:space:]]*$'
  falha ".git/info/exclude tem linha que nao e' padrao de ignorar (acima)"
else
  passa ".git limpo (info/exclude conferido: so padroes)"
fi

# -----------------------------------------------------------------------------
# 5. HISTORICO — todas as arvores, nao so' a de agora.
# -----------------------------------------------------------------------------
# As secoes 2 a 4 tem um furo que passou despercebido ate 2026-08-20:
#
#   secao 2  varre o WORKING TREE      -> nao ve historico
#   secao 3  varre MENSAGENS de commit -> nao ve conteudo de arquivo
#   secao 4  varre .git como arquivos  -> objetos sao ZLIB-COMPRIMIDOS, e o
#                                          grep nao enxerga dentro deles
#
# Resultado: um arquivo podia carregar o nome da ferramenta em cinco commits e o
# portao passava exit 0 todas as vezes — honestamente, porque nunca olhou la.
# Foi o mesmo padrao que ja tinha mordido a contagem das privilegiadas e o
# snapshot do IDL: varredura parcial passa, varredura de verdade pega.
#
# Esta secao percorre TODA arvore alcancavel e reprova enquanto houver sujeira.
# Depois de uma purga, e' ela que prova que sumiu.
# -----------------------------------------------------------------------------
#
# ⚠️ E DESDE 09/10 ELA VARRE A FAIXA NOVA, NAO `--all` (DECISOES 18:00). Varrer
# `--all` com o nome antigo no passado dava vermelho permanente — e vermelho
# permanente e vermelho que ninguem le. A varredura profunda continua; so mudou o
# alcance, e o alcance agora e o que este ramo acrescenta.
echo; hr; echo " 5. CONTEUDO NA FAIXA NOVA (arvore de cada commit)"; hr
COMMITS="$(git rev-list "$FAIXA" 2>/dev/null || true)"
if [ -z "$COMMITS" ]; then
  passa "sem historico para varrer"
else
  N_COMMITS="$(printf '%s\n' "$COMMITS" | wc -l | tr -d ' ')"
  # `git grep -lI` sobre varios commits devolve "commit:caminho". `-I` ignora
  # binario: objeto compactado nunca casaria, e sem isso o ruido esconderia o
  # que importa.
  # SEM `--` depois do padrao: `git grep PAT -- x` trata `x` como PATHSPEC, e a
  # varredura passaria a procurar arquivos CHAMADOS como os SHAs — devolvendo
  # vazio e dando verde falso. Foi assim que esta secao nasceu quebrada, e so'
  # apareceu porque ela foi testada contra sujeira conhecida antes de virar
  # portao. Trava que nunca falhou nao e trava.
  HITS="$(printf '%s\n' "$COMMITS" | xargs -r git grep -lIiE -e "$PAT" -- . "$ISENTOS" 2>/dev/null | sort -u || true)"
  if [ -n "$HITS" ]; then
    falha "ocorrencias no HISTORICO ($N_COMMITS commits varridos):"
    printf '%s\n' "$HITS" | while IFS=: read -r sha caminho; do
      printf '      %s  %s  %s\n' \
        "$(git rev-parse --short "$sha" 2>/dev/null)" \
        "$caminho" \
        "$(git log -1 --format='%s' "$sha" 2>/dev/null | cut -c1-48)"
    done
  else
    passa "historico limpo ($N_COMMITS commits varridos)"
  fi
fi

echo; hr
if [ "$FALHOU" -eq 0 ]; then
  printf '\033[1;32m AUTORIA CONFORME\033[0m\n'
else
  printf '\033[1;31m AUTORIA NAO CONFORME\033[0m\n'
fi
hr
exit "$FALHOU"
