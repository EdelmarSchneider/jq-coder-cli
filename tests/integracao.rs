//! Testes que tocam a rede / baixam o modelo real (~640 MB) — rodar com:
//!   cargo test --features integracao
#![cfg(feature = "integracao")]

#[test]
fn download_real_do_modelo_pinado() {
    // Usa o cache REAL do usuário de propósito: é o mesmo caminho que o
    // primeiro uso da CLI vai exercitar, e evita re-baixar 640 MB por teste.
    let caminho = jqc::modelo::garantir_modelo(jqc::modelo::REVISAO_PINADA, false)
        .expect("download do modelo pinado");
    let tamanho = std::fs::metadata(&caminho).expect("metadata").len();
    assert!(
        tamanho > 500_000_000,
        "GGUF Q8_0 tem ~640 MB, veio {tamanho}"
    );
}

#[test]
fn inferencia_real_gera_um_filtro_plausivel() {
    let gguf =
        jqc::modelo::garantir_modelo(jqc::modelo::REVISAO_PINADA, false).expect("modelo no cache");
    let mut motor = jqc::inferencia::carregar(&gguf, jqc::inferencia::Dispositivo::Cpu)
        .expect("carregar o GGUF");
    let mensagens = jqc::prompt::mensagens_de_inferencia(
        "get the id of every order",
        r#"{"orders": [{"id": 1, "status": "done"}]}"#,
    );
    let texto = motor.gerar(&mensagens).expect("geração");
    let filtro = jqc::prompt::extrair_programa(&texto);
    assert!(!filtro.is_empty(), "modelo devolveu vazio: {texto:?}");
    // Não fixamos o filtro exato aqui (isso é o Task 11); só que executa.
    let saida = jqc::executor::executar(&filtro, r#"{"orders": [{"id": 1, "status": "done"}]}"#)
        .expect("filtro gerado executa");
    assert!(saida.contains('1'), "saída inesperada: {saida}");
}

/// Os 4 exemplos publicados no model card, verificados POR EXECUÇÃO: o filtro
/// gerado roda contra o documento e a saída é comparada com a do filtro
/// publicado — igualdade de comportamento, não de string (greedy é estável,
/// mas string idêntica é mais frágil que o necessário).
#[test]
fn os_4_exemplos_do_model_card_passam_de_ponta_a_ponta() {
    let doc = r#"{"orders": [{"id": 1, "status": "done", "total": 120.5},
            {"id": 2, "status": "pending", "total": 40.0}]}"#;
    let casos = [
        ("get the id of every order", ".orders[] | .id"),
        (
            "keep only the orders whose status is done",
            "[.orders[] | select(.status == \"done\")]",
        ),
        (
            "some o total de todos os pedidos",
            "[.orders[].total] | add",
        ),
        (
            "remova o campo total de cada pedido",
            "del(.orders[].total)",
        ),
    ];
    let gguf =
        jqc::modelo::garantir_modelo(jqc::modelo::REVISAO_PINADA, false).expect("modelo no cache");
    let mut motor =
        jqc::inferencia::carregar(&gguf, jqc::inferencia::Dispositivo::Cpu).expect("carregar");
    for (pedido, filtro_publicado) in casos {
        let amostra = {
            let valor: serde_json::Value = serde_json::from_str(doc).expect("doc");
            jqc::prompt::podar_amostra(doc, &valor)
        };
        let mensagens = jqc::prompt::mensagens_de_inferencia(pedido, &amostra);
        let texto = motor.gerar(&mensagens).expect("geração");
        let filtro = jqc::prompt::extrair_programa(&texto);
        assert!(!filtro.is_empty(), "{pedido}: modelo devolveu vazio");
        let saida = jqc::executor::executar(&filtro, doc)
            .unwrap_or_else(|e| panic!("{pedido}: filtro {filtro:?} não executa: {e}"));
        let ouro = jqc::executor::executar(filtro_publicado, doc).expect("filtro publicado");
        assert_eq!(saida, ouro, "{pedido}: gerou {filtro:?}");
    }
}
