//! Compatibilidade do executor com o jq-bench (spec §6): os 30 programas-ouro
//! da fatia humana executados pelo NOSSO executor têm de reproduzir as
//! saídas-ouro (geradas com jq -cS). É o mesmo critério do spike do ADR-001,
//! agora contra o jaq EMBUTIDO em vez da CLI do jaq.
//!
//! Fonte: env JQC_BENCH (arquivo local) ou download do HF. Rodar com:
//!   cargo test --features integracao --test compat_bench
#![cfg(feature = "integracao")]

fn carregar_bench() -> String {
    if let Ok(local) = std::env::var("JQC_BENCH") {
        return std::fs::read_to_string(&local)
            .unwrap_or_else(|e| panic!("JQC_BENCH={local} ilegível: {e}"));
    }
    let url = "https://huggingface.co/datasets/DominuZ/jq-bench/resolve/main/humana.jsonl";
    let mut resposta = ureq::get(url).call().expect("baixar humana.jsonl do HF");
    resposta.body_mut().read_to_string().expect("corpo utf-8")
}

#[test]
fn executor_reproduz_os_30_programas_ouro_do_jq_bench() {
    let jsonl = carregar_bench();
    let mut pares = 0;
    let mut divergencias: Vec<String> = Vec::new();
    for linha in jsonl.lines().filter(|l| !l.trim().is_empty()) {
        let item: serde_json::Value = serde_json::from_str(linha).expect("linha do bench");
        let programa = item["programa"].as_str().expect("programa");
        let docs = item["documentos"].as_array().expect("documentos");
        let ouros = item["saidas"].as_array().expect("saidas");
        for (doc, ouro) in docs.iter().zip(ouros) {
            pares += 1;
            let doc = doc.as_str().expect("doc é string JSON");
            let ouro = ouro.as_str().expect("ouro é string").replace("\r\n", "\n");
            match jqc::executor::executar(programa, doc) {
                Ok(saida) if saida == ouro => {}
                Ok(saida) => divergencias.push(format!(
                    "{programa}\n  esperado: {ouro:?}\n  obtido:   {saida:?}"
                )),
                Err(erro) => divergencias.push(format!("{programa}\n  erro: {erro}")),
            }
        }
    }
    assert!(pares >= 60, "bench devia ter >= 60 pares, veio {pares}");
    assert!(
        divergencias.is_empty(),
        "{} de {pares} pares divergiram:\n{}",
        divergencias.len(),
        divergencias.join("\n")
    );
}
