//! Golden tests de paridade com o pipeline Python (regra do projeto: paridade
//! de contrato é testada, não presumida):
//! as fixtures foram exportadas por tools/exportar_fixtures.py rodando as
//! funções REAIS do repo JQ — qualquer divergência aqui é bug nosso.

use serde_json::Value;

fn fixture(nome: &str) -> Vec<Value> {
    let caminho = format!("{}/tests/fixtures/{nome}", env!("CARGO_MANIFEST_DIR"));
    let texto = std::fs::read_to_string(&caminho)
        .unwrap_or_else(|e| panic!("fixture {caminho} ilegível: {e}"));
    serde_json::from_str(&texto).unwrap_or_else(|e| panic!("fixture {caminho} inválida: {e}"))
}

#[test]
fn prompt_identico_ao_python() {
    let casos = fixture("prompt_casos.json");
    assert!(!casos.is_empty());
    for caso in casos {
        let pedido = caso["pedido_nl"].as_str().expect("pedido_nl");
        let amostra = caso["amostra_json"].as_str().expect("amostra_json");
        let [sistema, usuario] = jqc::prompt::mensagens_de_inferencia(pedido, amostra);
        assert_eq!(sistema.role, "system");
        assert_eq!(usuario.role, "user");
        assert_eq!(
            sistema.content,
            caso["system"].as_str().expect("system"),
            "system divergiu para {pedido:?}"
        );
        assert_eq!(
            usuario.content,
            caso["user"].as_str().expect("user"),
            "user divergiu para {pedido:?}"
        );
    }
}

#[test]
fn limpeza_identica_ao_python() {
    let casos = fixture("limpeza_casos.json");
    assert!(!casos.is_empty());
    for caso in casos {
        let entrada = caso["entrada"].as_str().expect("entrada");
        let esperado = caso["esperado"].as_str().expect("esperado");
        assert_eq!(
            jqc::prompt::extrair_programa(entrada),
            esperado,
            "limpeza divergiu para {entrada:?}"
        );
    }
}
