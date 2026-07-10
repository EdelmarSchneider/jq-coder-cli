//! O contrato de paridade com o pipeline Python do repo JQ.
//!
//! `mensagens_de_inferencia` replica `jqcoder.train.formatar` e
//! `extrair_programa` replica `jqcoder.data.professor.limpar_resposta` —
//! byte a byte, verificado pelos golden tests em tests/paridade.rs.

use std::sync::LazyLock;

use regex::Regex;

/// O prompt exato visto no treino — mudar UMA vírgula muda a distribuição.
const SISTEMA: &str =
    "You translate natural-language requests into jq filters. Reply with only the jq filter.";

// Regexes fixas: falha de compilação é impossível em runtime (testadas), por
// isso o expect não conta como panic em caminho de usuário.
static RE_THINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<think>.*?</think>").expect("regex fixa"));
static RE_THINK_TRUNCADO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<think>.*").expect("regex fixa"));

pub struct Mensagem {
    pub role: &'static str,
    pub content: String,
}

pub fn mensagens_de_inferencia(pedido_nl: &str, amostra_json: &str) -> [Mensagem; 2] {
    [
        Mensagem {
            role: "system",
            content: SISTEMA.to_string(),
        },
        Mensagem {
            role: "user",
            content: format!("{pedido_nl}\n\nJSON sample:\n{amostra_json}"),
        },
    ]
}

/// Porte fiel de `limpar_resposta` (jqcoder.data.professor): remove blocos
/// <think> (fechados OU truncados), cercas de código e aspas externas.
/// Fiel inclui os cantos estranhos — `"foo" + "bar"` vira `foo" + "bar` lá
/// e aqui; paridade vale mais que gosto.
pub fn extrair_programa(texto_gerado: &str) -> String {
    let sem_think = RE_THINK.replace_all(texto_gerado, "");
    let sem_think = RE_THINK_TRUNCADO.replace_all(&sem_think, "");
    let mut limpo = sem_think.trim().to_string();
    if limpo.starts_with("```") {
        limpo = limpo
            .lines()
            .filter(|linha| !linha.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
    }
    let caracteres: Vec<char> = limpo.chars().collect();
    if caracteres.len() >= 2 {
        let (primeiro, ultimo) = (caracteres[0], caracteres[caracteres.len() - 1]);
        if primeiro == ultimo && (primeiro == '"' || primeiro == '\'') {
            limpo = caracteres[1..caracteres.len() - 1]
                .iter()
                .collect::<String>()
                .trim()
                .to_string();
        }
    }
    limpo
}

/// Acima disso a amostra é podada (spec §2: "~6 KB"): o modelo precisa da
/// FORMA do documento, não do conteúdo — foi treinado com documentos pequenos.
pub const LIMITE_AMOSTRA_BYTES: usize = 6144;

const MAX_ITENS_ARRAY: usize = 3;

/// Documento pequeno entra cru (paridade com a CLI Python); grande é podado
/// estruturalmente: arrays cortados aos 3 primeiros elementos, recursivamente.
/// O filtro sempre executa contra o documento COMPLETO — a poda é só do prompt.
pub fn podar_amostra(documento_texto: &str, documento: &serde_json::Value) -> String {
    if documento_texto.len() <= LIMITE_AMOSTRA_BYTES {
        return documento_texto.to_string();
    }
    let mut podado = documento.clone();
    podar(&mut podado);
    podado.to_string()
}

fn podar(valor: &mut serde_json::Value) {
    match valor {
        serde_json::Value::Array(itens) => {
            itens.truncate(MAX_ITENS_ARRAY);
            itens.iter_mut().for_each(podar);
        }
        serde_json::Value::Object(campos) => campos.values_mut().for_each(podar),
        _ => {}
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn amostra_pequena_vai_verbatim() {
        // Paridade com a CLI Python: documento pequeno entra CRU no prompt,
        // espaçamento e ordem de chaves preservados.
        let texto = "{ \"b\" :  1 ,\n  \"a\": [ 1, 2, 3, 4 ] }";
        let valor: serde_json::Value = serde_json::from_str(texto).expect("json de teste");
        assert_eq!(podar_amostra(texto, &valor), texto);
    }

    #[test]
    fn amostra_no_limite_vai_verbatim() {
        let itens: Vec<String> = (0..1)
            .map(|_| format!("\"{}\"", "x".repeat(LIMITE_AMOSTRA_BYTES - 2)))
            .collect();
        let texto = itens[0].clone(); // exatamente LIMITE_AMOSTRA_BYTES bytes
        assert_eq!(texto.len(), LIMITE_AMOSTRA_BYTES);
        let valor: serde_json::Value = serde_json::from_str(&texto).expect("json de teste");
        assert_eq!(podar_amostra(&texto, &valor), texto);
    }

    #[test]
    fn amostra_grande_corta_arrays_a_3_recursivamente() {
        // 500 objetos, cada um com um array interno de 10 — só a FORMA importa
        // para o modelo (treinado com documentos pequenos).
        let interno: Vec<u32> = (0..10).collect();
        let doc: Vec<serde_json::Value> = (0..500)
            .map(|i| serde_json::json!({"id": i, "valores": interno}))
            .collect();
        let texto = serde_json::to_string(&doc).expect("json de teste");
        assert!(texto.len() > LIMITE_AMOSTRA_BYTES);
        let valor: serde_json::Value = serde_json::from_str(&texto).expect("json de teste");
        let amostra = podar_amostra(&texto, &valor);
        let podado: serde_json::Value = serde_json::from_str(&amostra).expect("amostra é JSON");
        let externo = podado.as_array().expect("array externo");
        assert_eq!(externo.len(), 3);
        for item in externo {
            assert_eq!(item["valores"].as_array().expect("array interno").len(), 3);
        }
    }

    #[test]
    fn amostra_grande_poda_arrays_dentro_de_objetos() {
        let doc =
            serde_json::json!({"meta": {"tags": (0..2000).collect::<Vec<u32>>()}, "nome": "x"});
        let texto = serde_json::to_string(&doc).expect("json de teste");
        assert!(texto.len() > LIMITE_AMOSTRA_BYTES);
        let amostra = podar_amostra(&texto, &doc);
        let podado: serde_json::Value = serde_json::from_str(&amostra).expect("amostra é JSON");
        assert_eq!(podado["meta"]["tags"].as_array().expect("tags").len(), 3);
        assert_eq!(podado["nome"], "x");
    }
}
