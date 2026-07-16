//! Gravação segura no arquivo do usuário (spec §"Modo um-comando").
//!
//! Invariante central: o ÚNICO ato irreversível do jqc é o rename final da
//! gravação atômica — todo o resto (validação, diff, confirmação) existe
//! para chegar lá sem surpresa. A saída de uma execução só é gravável se
//! for exatamente UM documento JSON: um stream de 2+ valores ou uma saída
//! vazia não têm forma de arquivo.

use std::io::{BufRead, Write};

#[derive(Debug, thiserror::Error)]
pub enum ErroGravacao {
    #[error("the filter produced no output — nothing to write")]
    SaidaVazia,
    #[error("the filter produced a stream of {0} values, not a single document — cannot write")]
    SaidaMultipla(usize),
    #[error("the filter output is not valid JSON: {0}")]
    SaidaInvalida(String),
    #[error("could not write the file: {0}")]
    Io(#[from] std::io::Error),
}

/// A saída canônica do executor (um valor por linha) só é gravável se
/// contiver exatamente UM documento. `StreamDeserializer` conta os valores
/// sem depender de contagem de linhas (strings com \n embutido são um valor só).
pub fn documento_unico(saida_execucao: &str) -> Result<serde_json::Value, ErroGravacao> {
    let mut stream =
        serde_json::Deserializer::from_str(saida_execucao).into_iter::<serde_json::Value>();
    let primeiro = match stream.next() {
        None => return Err(ErroGravacao::SaidaVazia),
        Some(Err(erro)) => return Err(ErroGravacao::SaidaInvalida(erro.to_string())),
        Some(Ok(valor)) => valor,
    };
    // Falhar alto no tail: um erro de parse depois do primeiro documento
    // significa saída malformada — engolir isso e gravar seria trair o gate.
    let mut restantes = 0usize;
    for item in stream {
        match item {
            Err(erro) => return Err(ErroGravacao::SaidaInvalida(erro.to_string())),
            Ok(_) => restantes += 1,
        }
    }
    if restantes > 0 {
        return Err(ErroGravacao::SaidaMultipla(1 + restantes));
    }
    Ok(primeiro)
}

/// Pretty-print de 2 espaços + newline final: forma legível e determinística.
/// serde_json::Value ordena chaves (Map = BTreeMap) — coerente com o canon
/// `jq -S` que o resto do projeto usa.
pub fn formatar_para_arquivo(doc: &serde_json::Value) -> String {
    let mut texto = serde_json::to_string_pretty(doc).unwrap_or_else(|_| doc.to_string());
    texto.push('\n');
    texto
}

/// Diff posicional linha a linha (não é LCS — para JSON canônico pretty dos
/// dois lados, divergência posicional é o que interessa; um diff estrutural
/// seria dependência nova por pouco ganho). Vazio ⇢ conteúdos idênticos.
pub fn diff_resumido(atual: &str, proposto: &str, max_divergencias: usize) -> String {
    let a: Vec<&str> = atual.lines().collect();
    let p: Vec<&str> = proposto.lines().collect();
    let mut linhas = Vec::new();
    let mut divergencias = 0usize;
    let mut ocultas = 0usize;
    for i in 0..a.len().max(p.len()) {
        let la = a.get(i);
        let lp = p.get(i);
        if la == lp {
            continue;
        }
        if divergencias < max_divergencias {
            if let Some(l) = la {
                linhas.push(format!("- {l}"));
            }
            if let Some(l) = lp {
                linhas.push(format!("+ {l}"));
            }
        } else {
            ocultas += 1;
        }
        divergencias += 1;
    }
    if ocultas > 0 {
        linhas.push(format!("… ({ocultas} more differing lines)"));
    }
    linhas.join("\n")
}

/// Pergunta e lê UMA linha; só `y`/`yes`/`s`/`sim` (case-insensitive) valem
/// sim — default é NÃO, como todo gate de gravação deve ser.
pub fn confirmar<R: BufRead, W: Write>(entrada: &mut R, saida: &mut W, pergunta: &str) -> bool {
    let _ = write!(saida, "{pergunta}");
    let _ = saida.flush();
    let mut linha = String::new();
    if entrada.read_line(&mut linha).is_err() {
        return false;
    }
    matches!(
        linha.trim().to_lowercase().as_str(),
        "y" | "yes" | "s" | "sim"
    )
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn um_documento_e_gravavel() {
        let doc = documento_unico("{\"a\":1}").expect("um doc");
        assert_eq!(doc["a"], 1);
    }

    #[test]
    fn stream_de_dois_valores_nao_e_gravavel() {
        assert!(matches!(
            documento_unico("1\n2"),
            Err(ErroGravacao::SaidaMultipla(2))
        ));
    }

    #[test]
    fn saida_vazia_nao_e_gravavel() {
        assert!(matches!(documento_unico(""), Err(ErroGravacao::SaidaVazia)));
        assert!(matches!(
            documento_unico("  \n"),
            Err(ErroGravacao::SaidaVazia)
        ));
    }

    #[test]
    fn lixo_nao_e_gravavel() {
        assert!(matches!(
            documento_unico("not json"),
            Err(ErroGravacao::SaidaInvalida(_))
        ));
    }

    #[test]
    fn lixo_apos_o_documento_nao_e_gravavel() {
        assert!(matches!(
            documento_unico("1\ngarbage"),
            Err(ErroGravacao::SaidaInvalida(_))
        ));
    }

    #[test]
    fn lixo_apos_stream_valido_tambem_falha() {
        assert!(matches!(
            documento_unico("1\n2\nbad"),
            Err(ErroGravacao::SaidaInvalida(_))
        ));
    }

    #[test]
    fn formatar_e_pretty_com_newline_final() {
        let doc: serde_json::Value = serde_json::from_str("{\"b\":2,\"a\":1}").expect("json");
        let texto = formatar_para_arquivo(&doc);
        assert!(texto.ends_with('\n'));
        assert!(texto.contains("  \"a\": 1"));
    }

    #[test]
    fn diff_mostra_linhas_divergentes_com_prefixo() {
        let d = diff_resumido("{\n  \"a\": 1\n}", "{\n  \"a\": 2\n}", 10);
        assert!(d.contains("- "));
        assert!(d.contains("+ "));
        assert!(d.contains("\"a\": 1"));
        assert!(d.contains("\"a\": 2"));
    }

    #[test]
    fn diff_iguais_e_vazio() {
        assert_eq!(diff_resumido("x", "x", 10), "");
    }

    #[test]
    fn diff_respeita_o_teto_e_anuncia_o_resto() {
        let atual: String = (0..50).map(|i| format!("{i}\n")).collect();
        let proposto: String = (0..50).map(|i| format!("{}\n", i + 1000)).collect();
        let d = diff_resumido(&atual, &proposto, 5);
        assert!(d.contains("more differing lines"));
        assert_eq!(d.matches("- ").count(), 5);
    }

    #[test]
    fn confirmar_aceita_y_e_s_e_recusa_o_resto() {
        use std::io::Cursor;
        for (resposta, esperado) in [
            ("y\n", true),
            ("s\n", true),
            ("Y\n", true),
            ("n\n", false),
            ("\n", false),
            ("yes\n", true),
        ] {
            let mut entrada = Cursor::new(resposta.as_bytes().to_vec());
            let mut saida = Vec::new();
            assert_eq!(
                confirmar(&mut entrada, &mut saida, "write changes? [y/N] "),
                esperado,
                "resposta {resposta:?}"
            );
        }
        let mut entrada = Cursor::new(b"y\n".to_vec());
        let mut saida = Vec::new();
        confirmar(&mut entrada, &mut saida, "write changes? [y/N] ");
        assert!(String::from_utf8_lossy(&saida).contains("write changes?"));
    }
}
