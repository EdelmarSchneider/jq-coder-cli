# Decisões de arquitetura (ADRs) — jqc

Formato curto: contexto → decisão → evidência → consequências. Um ADR só muda
por outro ADR.

---

## ADR-001 — Executor de filtros: jaq embutido como biblioteca (2026-07-10)

**Contexto.** O design aprovado exigia decidir o executor **antes** de qualquer
código da CLI, por medição contra o jq-bench — nunca por suposição. Critério binário
pré-combinado: qualquer divergência num programa-ouro da fatia humana reprova o
jaq e ativa o plano B (jq oficial embarcado como asset + subprocess).

**Decisão.** **jaq aprovado** — será embutido como biblioteca
(`jaq-core`/`jaq-json`, família 3.x): binário único puro, sem assets externos.

**Evidência** (spike reproduzível em `spikes/jaq_vs_jq/spike.py`; resultado
bruto em `spikes/jaq_vs_jq/resultado.json`):

- jaq 3.1.0 (binário de release), controle jq 1.8.2, Windows x64, 2026-07-10.
- **Fatia humana (critério): 30 programas, 60 pares documento×saída — 60/60
  idênticos byte a byte** ao ouro (`jq -cS`).
- Fatia in-dist (informativa): 200 itens amostrados de `bulk-0011/train.jsonl`,
  1000 pares — 995/1000. As 5 divergências têm **uma única causa-raiz**:
  `join(sep)` com elemento `null` — jq converte `null` em `""`
  (`["a",null] | join(",")` → `"a,"`), jaq imprime `"null"` → `"a,null"`.
  Diferença semântica conhecida do jaq, fora da fatia-critério.

**Armadilha registrada durante o spike.** `jq.exe` no Windows emite `\r\n`
(stdout em modo texto); jaq emite `\n`. A comparação byte a byte precisa
normalizar quebras de linha — formatação, não semântica. Sem isso, o controle
jq "falha" contra o próprio ouro que gerou.

**Consequências.**

1. `executor.rs` usa `jaq-core`/`jaq-json` in-process; a interface
   `executar(filtro, json) -> Result<String>` mantém a troca local caso o plano
   B precise ser ativado um dia.
2. A regra "timeout SEMPRE" continua valendo: jaq in-process não é matável, a
   CLI re-invoca a si mesma num subprocesso oculto com timeout+kill.
3. Limitação aceita e documentada: filtros gerados pelo modelo que usem
   `join` sobre valores `null` podem divergir do jq oficial. Frequência
   observada: 0,5% dos pares in-dist, 0% na fatia humana.
4. **Qualquer troca futura de executor (ou upgrade de versão do jaq) re-roda
   este spike** — o critério é medido, não presumido (regra inegociável do
   projeto; o teste `compat_bench` re-executa o mesmo critério no CI).
