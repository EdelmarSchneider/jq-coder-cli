//! Sessão interativa (spec §"Modo REPL"): buffer explícito — resultado é
//! visualização até `:a`; o arquivo só muda em `:w`. O loop (Task 6) é
//! genérico sobre IO e sobre as closures de geração/execução para que TODA
//! esta lógica teste sem modelo, sem subprocesso e sem terminal.

use crate::gravar;

pub enum Comando {
    Pedido(String),
    Aplicar,
    Desfazer,
    Gravar,
    Sair,
    Ajuda,
    Vazio,
    Desconhecido(String),
}

pub fn parse_comando(linha: &str) -> Comando {
    let aparado = linha.trim();
    match aparado {
        "" => Comando::Vazio,
        ":a" => Comando::Aplicar,
        ":d" => Comando::Desfazer,
        ":w" => Comando::Gravar,
        ":q" => Comando::Sair,
        ":?" => Comando::Ajuda,
        _ if aparado.starts_with(':') => Comando::Desconhecido(aparado.to_string()),
        _ => Comando::Pedido(aparado.to_string()),
    }
}

/// Buffer de trabalho: texto (vai no prompt e na execução) + valor parseado
/// (vai na poda da amostra). Os dois andam SEMPRE juntos — o único jeito de
/// mudá-los é `aplicar`, que valida antes.
pub struct Sessao {
    texto: String,
    valor: serde_json::Value,
    ultimo_resultado: Option<String>,
    undo: Vec<(String, serde_json::Value)>,
}

impl Sessao {
    pub fn nova(texto: String) -> Result<Sessao, String> {
        let valor: serde_json::Value =
            serde_json::from_str(&texto).map_err(|erro| format!("invalid JSON: {erro}"))?;
        Ok(Sessao {
            texto,
            valor,
            ultimo_resultado: None,
            undo: Vec::new(),
        })
    }

    pub fn buffer_texto(&self) -> &str {
        &self.texto
    }

    pub fn buffer_valor(&self) -> &serde_json::Value {
        &self.valor
    }

    pub fn registrar_resultado(&mut self, saida: String) {
        self.ultimo_resultado = Some(saida);
    }

    /// `:a` — o último resultado vira o buffer, SE for um documento único.
    pub fn aplicar(&mut self) -> Result<(), String> {
        let Some(saida) = self.ultimo_resultado.as_deref() else {
            return Err("nothing to apply — run a request first".to_string());
        };
        let doc = gravar::documento_unico(saida).map_err(|erro| erro.to_string())?;
        let novo_texto = gravar::formatar_para_arquivo(&doc);
        self.undo.push((
            std::mem::take(&mut self.texto),
            std::mem::replace(&mut self.valor, doc),
        ));
        self.texto = novo_texto;
        Ok(())
    }

    /// `:d` — volta um estado da pilha (pilha completa da sessão).
    pub fn desfazer(&mut self) -> Result<(), String> {
        let Some((texto, valor)) = self.undo.pop() else {
            return Err("nothing to undo".to_string());
        };
        self.texto = texto;
        self.valor = valor;
        Ok(())
    }

    /// O buffer difere do conteúdo do arquivo? (comparação semântica: os
    /// dois reserializados; espaçamento não conta como mudança)
    pub fn alterado(&self, conteudo_arquivo: &str) -> bool {
        match serde_json::from_str::<serde_json::Value>(conteudo_arquivo) {
            Ok(doc_arquivo) => doc_arquivo != self.valor,
            Err(_) => true,
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn parse_reconhece_todos_os_comandos() {
        assert!(matches!(parse_comando(":a"), Comando::Aplicar));
        assert!(matches!(parse_comando(" :d "), Comando::Desfazer));
        assert!(matches!(parse_comando(":w"), Comando::Gravar));
        assert!(matches!(parse_comando(":q"), Comando::Sair));
        assert!(matches!(parse_comando(":?"), Comando::Ajuda));
        assert!(matches!(parse_comando(""), Comando::Vazio));
        assert!(matches!(parse_comando(":zz"), Comando::Desconhecido(_)));
        assert!(matches!(
            parse_comando("remova o total"),
            Comando::Pedido(_)
        ));
    }

    #[test]
    fn aplicar_sem_resultado_e_erro() {
        let mut s = Sessao::nova("{\"a\":1}".to_string()).expect("json");
        assert!(s.aplicar().is_err());
    }

    #[test]
    fn aplicar_troca_o_buffer_e_desfazer_volta() {
        let mut s = Sessao::nova("{\"a\":1}".to_string()).expect("json");
        s.registrar_resultado("{\"a\":2}".to_string());
        s.aplicar().expect("aplicavel");
        assert_eq!(s.buffer_valor()["a"], 2);
        s.desfazer().expect("tem undo");
        assert_eq!(s.buffer_valor()["a"], 1);
        assert!(s.desfazer().is_err(), "pilha vazia");
    }

    #[test]
    fn aplicar_recusa_stream() {
        let mut s = Sessao::nova("[1,2]".to_string()).expect("json");
        s.registrar_resultado("1\n2".to_string());
        assert!(s.aplicar().is_err());
        assert_eq!(s.buffer_texto(), "[1,2]", "buffer intacto após recusa");
    }

    #[test]
    fn alterado_e_semantico() {
        let mut s = Sessao::nova("{\"a\":1}".to_string()).expect("json");
        assert!(!s.alterado("{ \"a\" : 1 }"), "espaçamento não é mudança");
        s.registrar_resultado("{\"a\":2}".to_string());
        s.aplicar().expect("aplicavel");
        assert!(s.alterado("{\"a\":1}"));
    }
}
