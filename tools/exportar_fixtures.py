"""Exporta fixtures de paridade do pipeline Python (repo JQ) para o jqc (Rust).

Regra do projeto jqc: paridade de contrato é TESTADA, não presumida.
Este script roda DENTRO do repo JQ (uv de lá) e grava a verdade do Python:
- prompt_casos.json: (pedido, amostra) -> conteúdos system/user EXATOS
- limpeza_casos.json: texto bruto do modelo -> filtro limpo EXATO

Uso (a partir de F:\\AIProjects\\JQ):
  PYTHONIOENCODING=utf-8 uv run python F:/aiprojects/jqc/tools/exportar_fixtures.py \
      --saida F:/aiprojects/jqc/tests/fixtures
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from jqcoder.data.professor import limpar_resposta
from jqcoder.train.formatar import mensagens_de_inferencia

CASOS_PROMPT: list[tuple[str, str]] = [
    ("get the id of every order", '{"orders": [{"id": 1, "status": "done"}]}'),
    ("some o total de todos os pedidos", '{"pedidos": [{"total": 120.5}, {"total": 40.0}]}'),
    ("remova o campo endereço de cada usuário", '{"usuários": [{"nome": "João", "endereço": "rua á"}]}'),
    ("count items", "[]"),
    ("pedido com \"aspas\" e 'apóstrofos'", '{"a": 1}'),
    ("pedido\ncom quebra de linha", '{"a": 1}'),
    ("keep unicode intact", '{"emoji": "🎉", "cjk": "亿", "tab": "a\\tb"}'),
    ("amostra com espaçamento estranho", '{ "a" :  1 ,\n  "b": [ 1, 2 ] }'),
]

CASOS_LIMPEZA: list[str] = [
    ".foo",
    "  .foo | .bar  ",
    "<think>raciocínio</think>.foo",
    "<think>multi\nlinha</think>\n.foo | length",
    "<think>truncado sem fechar .foo",
    "```jq\n.foo\n```",
    "```\n.foo | select(.a == \"b\")\n```",
    "```jq\nlinha1 |\nlinha2\n```",
    '".foo"',
    "'.foo'",
    '"',
    '""',
    '"foo" + "bar"',
    "<think>a</think>```jq\n\".foo\"\n```",
    "`.foo`",
    "",
    "   ",
    ".foo # comentário unicode ção",
]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--saida", type=Path, required=True)
    args = parser.parse_args()
    args.saida.mkdir(parents=True, exist_ok=True)

    prompt_casos = []
    for pedido, amostra in CASOS_PROMPT:
        mensagens = mensagens_de_inferencia(pedido, amostra)
        assert [m["role"] for m in mensagens] == ["system", "user"]
        prompt_casos.append(
            {
                "pedido_nl": pedido,
                "amostra_json": amostra,
                "system": mensagens[0]["content"],
                "user": mensagens[1]["content"],
            }
        )
    limpeza_casos = [{"entrada": t, "esperado": limpar_resposta(t)} for t in CASOS_LIMPEZA]

    for nome, dados in [("prompt_casos.json", prompt_casos), ("limpeza_casos.json", limpeza_casos)]:
        destino = args.saida / nome
        destino.write_text(
            json.dumps(dados, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
            newline="\n",
        )
        print(f"gravado: {destino} ({len(dados)} casos)")


if __name__ == "__main__":
    main()
