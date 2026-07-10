# Spike jaq × jq-bench (spec §4) — decide o executor do jqc ANTES da CLI.
#
# Para cada item do bench, cada par (documento, saída-ouro) é executado com
# `jaq -cS` e comparado byte a byte com a saída-ouro (gerada com `jq -cS` na
# fábrica). Como controle, o mesmo par roda no jq oficial: se o CONTROLE
# divergir, o problema é ambiente/versão do jq, não o jaq.
#
# Critério binário (spec): qualquer divergência num programa-ouro HUMANO
# reprova o jaq. Os itens in-dist são informativos (mesma distribuição do
# treino, não fazem parte do critério).
#
# Uso:
#   python spike.py --jaq <jaq.exe> --jq <jq.exe> \
#       --humana F:/AIProjects/JQ/data/bench/humana.jsonl \
#       --train F:/AIProjects/JQ/data/generated/bulk-0011/train.jsonl \
#       --n-train 200 --out resultado.json

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

TIMEOUT_S = 15


def executar(binario: str, programa: str, documento: str) -> tuple[str, str]:
    """Roda `binario -cS <programa>` com o documento no stdin.

    Devolve (status, saida): status é "ok", "erro" ou "timeout"; saida é o
    stdout sem o \n final (ou a primeira linha do stderr, para diagnóstico).
    """
    try:
        proc = subprocess.run(
            [binario, "-cS", programa],
            input=documento.encode("utf-8"),
            capture_output=True,
            timeout=TIMEOUT_S,
        )
    except subprocess.TimeoutExpired:
        return "timeout", ""
    if proc.returncode != 0:
        stderr = proc.stderr.decode("utf-8", errors="replace").strip()
        return "erro", stderr.splitlines()[0] if stderr else ""
    # jq.exe no Windows emite \r\n (stdout em modo texto); o ouro foi gravado
    # com \n. Normalizar quebras é formatação, não semântica.
    saida = proc.stdout.decode("utf-8", errors="replace").replace("\r\n", "\n")
    return "ok", saida.rstrip("\n")


def amostrar_train(caminho: Path, n: int) -> list[dict]:
    """Amostra determinística: ~n linhas espaçadas uniformemente no arquivo."""
    with caminho.open(encoding="utf-8") as f:
        total = sum(1 for _ in f)
    passo = max(1, total // n)
    itens = []
    with caminho.open(encoding="utf-8") as f:
        for i, linha in enumerate(f):
            if i % passo == 0 and len(itens) < n:
                itens.append(json.loads(linha))
    return itens


def avaliar(itens: list[dict], fatia: str, jaq: str, jq: str) -> dict:
    resumo = {
        "fatia": fatia,
        "itens": len(itens),
        "pares": 0,
        "jaq_ok": 0,
        "controle_jq_falhou": 0,  # ouro irreprodutível até com jq — ambiente
        "divergencias": [],
    }
    for idx, item in enumerate(itens):
        programa = item["programa"]
        for doc, ouro in zip(item["documentos"], item["saidas"]):
            resumo["pares"] += 1
            status, saida = executar(jaq, programa, doc)
            if status == "ok" and saida == ouro:
                resumo["jaq_ok"] += 1
                continue
            # jaq divergiu — o controle jq reproduz o ouro?
            status_jq, saida_jq = executar(jq, programa, doc)
            controle_ok = status_jq == "ok" and saida_jq == ouro
            if not controle_ok:
                resumo["controle_jq_falhou"] += 1
            resumo["divergencias"].append(
                {
                    "familia": item.get("familia", f"{fatia}[{idx}]"),
                    "programa": programa,
                    "jaq_status": status,
                    "jaq_saida": saida[:400],
                    "ouro": ouro[:400],
                    "controle_jq_reproduz_ouro": controle_ok,
                }
            )
    return resumo


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--jaq", required=True)
    ap.add_argument("--jq", required=True)
    ap.add_argument("--humana", required=True, type=Path)
    ap.add_argument("--train", type=Path)
    ap.add_argument("--n-train", type=int, default=200)
    ap.add_argument("--out", type=Path, default=Path("resultado.json"))
    args = ap.parse_args()

    humana = [
        json.loads(l) for l in args.humana.read_text(encoding="utf-8").splitlines() if l
    ]
    resultados = [avaliar(humana, "humana", args.jaq, args.jq)]

    if args.train:
        train = amostrar_train(args.train, args.n_train)
        resultados.append(avaliar(train, "in-dist", args.jaq, args.jq))

    args.out.write_text(
        json.dumps(resultados, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    for r in resultados:
        reais = [d for d in r["divergencias"] if d["controle_jq_reproduz_ouro"]]
        print(
            f"[{r['fatia']}] itens={r['itens']} pares={r['pares']} "
            f"jaq_ok={r['jaq_ok']} divergencias={len(r['divergencias'])} "
            f"(reais={len(reais)}, ambiente={r['controle_jq_falhou']})"
        )

    humana_reais = [
        d for d in resultados[0]["divergencias"] if d["controle_jq_reproduz_ouro"]
    ]
    veredito = "REPROVADO" if humana_reais else "APROVADO"
    print(f"\nVeredito (criterio binario, fatia humana): jaq {veredito}")
    return 1 if humana_reais else 0


if __name__ == "__main__":
    sys.exit(main())
