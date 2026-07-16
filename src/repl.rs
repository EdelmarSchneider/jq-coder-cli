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
    /// Consome `ultimo_resultado` SÓ em caso de sucesso: uma recusa (stream,
    /// vazio, JSON inválido) mantém o resultado disponível para nova tentativa,
    /// mas um `:a` bem-sucedido some com ele — senão um segundo `:a` sem pedido
    /// novo reaplicaria o mesmo resultado e empilharia um undo duplicado.
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
        self.ultimo_resultado = None;
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

const AJUDA: &str = "commands: :a apply last result to the buffer · :d undo · \
:w write buffer to the file · :q quit · :? this help\nanything else is a natural-language request";

/// O loop interativo (spec §"Modo REPL"). Genérico sobre IO (`R`/`W`) e sobre
/// as closures de geração/execução (`G`/`E`) para testar toda a lógica de
/// comando com fakes determinísticos — sem modelo, sem subprocesso, sem
/// terminal de verdade. `main.rs` (Task 7) chama isto com stdin/stdout reais
/// e as closures que envolvem o motor de inferência e o executor jq/jaq.
pub fn rodar_sessao<R, W, G, E>(
    entrada: &mut R,
    saida: &mut W,
    arquivo: &std::path::Path,
    texto_inicial: &str,
    gerar: &mut G,
    executar: &E,
) -> i32
where
    R: std::io::BufRead,
    W: std::io::Write,
    G: FnMut(&str, &str) -> Result<String, String>,
    E: Fn(&str, &str) -> Result<String, String>,
{
    let mut sessao = match Sessao::nova(texto_inicial.to_string()) {
        Ok(sessao) => sessao,
        Err(erro) => {
            let _ = writeln!(saida, "{erro}");
            return 2;
        }
    };
    let _ = writeln!(saida, "jqc interactive — :? for help");
    let mut avisado_de_sair = false;
    loop {
        let _ = write!(saida, "jqc> ");
        let _ = saida.flush();
        let mut linha = String::new();
        match entrada.read_line(&mut linha) {
            Ok(0) | Err(_) => {
                // EOF: ao contrário de :q, não há como reperguntar — avisa
                // (se houver buffer não gravado) e encerra sem bloquear.
                let atual = std::fs::read_to_string(arquivo).unwrap_or_default();
                if sessao.alterado(&atual) {
                    let _ = writeln!(saida, "unsaved changes discarded (EOF)");
                }
                return 0;
            }
            Ok(_) => {}
        }
        let comando = parse_comando(&linha);
        if !matches!(comando, Comando::Sair) {
            avisado_de_sair = false; // qualquer outra ação rearma o aviso
        }
        match comando {
            Comando::Vazio => {}
            Comando::Ajuda => {
                let _ = writeln!(saida, "{AJUDA}");
            }
            Comando::Desconhecido(qual) => {
                let _ = writeln!(saida, "unknown command {qual} — :? for help");
            }
            Comando::Pedido(pedido) => {
                let amostra =
                    crate::prompt::podar_amostra(sessao.buffer_texto(), sessao.buffer_valor());
                match gerar(&pedido, &amostra) {
                    Err(erro) => {
                        let _ = writeln!(saida, "{erro}");
                    }
                    Ok(filtro) => {
                        let _ = writeln!(saida, "filter: {filtro}");
                        match executar(&filtro, sessao.buffer_texto()) {
                            Ok(resultado) => {
                                let _ = writeln!(saida, "{resultado}");
                                sessao.registrar_resultado(resultado);
                            }
                            Err(erro) => {
                                let _ = writeln!(saida, "{erro}");
                            }
                        }
                    }
                }
            }
            Comando::Aplicar => {
                if let Err(erro) = sessao.aplicar() {
                    let _ = writeln!(saida, "{erro}");
                } else {
                    let _ = writeln!(saida, "applied — buffer updated (:d to undo)");
                }
            }
            Comando::Desfazer => {
                if let Err(erro) = sessao.desfazer() {
                    let _ = writeln!(saida, "{erro}");
                } else {
                    let _ = writeln!(saida, "undone");
                }
            }
            Comando::Gravar => {
                let atual = std::fs::read_to_string(arquivo).unwrap_or_default();
                let diff = gravar::diff_resumido(&atual, sessao.buffer_texto(), 40);
                if diff.is_empty() {
                    let _ = writeln!(saida, "no changes — file left untouched");
                    continue;
                }
                let _ = writeln!(saida, "--- {} (on disk)", arquivo.display());
                let _ = writeln!(saida, "+++ buffer");
                let _ = writeln!(saida, "{diff}");
                if !gravar::confirmar(entrada, saida, "write changes? [y/N] ") {
                    let _ = writeln!(saida, "aborted — file left untouched");
                    continue;
                }
                match gravar::gravar_atomico(arquivo, sessao.buffer_texto()) {
                    Ok(bak) => {
                        if bak.exists() {
                            let _ = writeln!(
                                saida,
                                "written; previous version kept at {}",
                                bak.display()
                            );
                        } else {
                            let _ = writeln!(saida, "written (no previous version to back up)");
                        }
                    }
                    Err(erro) => {
                        let _ = writeln!(saida, "{erro}");
                    }
                }
            }
            Comando::Sair => {
                let atual = std::fs::read_to_string(arquivo).unwrap_or_default();
                if sessao.alterado(&atual) && !avisado_de_sair {
                    let _ = writeln!(saida, "unsaved changes — :w to save, :q again to discard");
                    avisado_de_sair = true;
                    continue;
                }
                return 0;
            }
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
    fn aplicar_consome_o_resultado() {
        let mut s = Sessao::nova("{\"a\":1}".to_string()).expect("json");
        s.registrar_resultado("{\"a\":2}".to_string());
        s.aplicar().expect("primeira aplicacao");
        assert!(
            s.aplicar().is_err(),
            "segunda :a sem novo pedido deve falhar"
        );
    }

    #[test]
    fn alterado_e_semantico() {
        let mut s = Sessao::nova("{\"a\":1}".to_string()).expect("json");
        assert!(!s.alterado("{ \"a\" : 1 }"), "espaçamento não é mudança");
        s.registrar_resultado("{\"a\":2}".to_string());
        s.aplicar().expect("aplicavel");
        assert!(s.alterado("{\"a\":1}"));
    }

    fn rodar_com(script: &str, texto: &str, arquivo: &std::path::Path) -> (i32, String) {
        let mut entrada = std::io::Cursor::new(script.as_bytes().to_vec());
        let mut saida = Vec::new();
        // Fake determinístico: pedido "dobra" vira um filtro fixo; o executor
        // fake devolve o documento com "a" dobrado, sem jaq nem subprocesso.
        let mut gerar = |pedido: &str, _amostra: &str| -> Result<String, String> {
            if pedido == "dobra" {
                Ok(".a *= 2".to_string())
            } else {
                Err("unknown".to_string())
            }
        };
        let executar = |_filtro: &str, doc: &str| -> Result<String, String> {
            let v: serde_json::Value = serde_json::from_str(doc).map_err(|e| e.to_string())?;
            let a = v["a"].as_i64().unwrap_or(0);
            Ok(format!("{{\"a\":{}}}", a * 2))
        };
        let codigo = rodar_sessao(
            &mut entrada,
            &mut saida,
            arquivo,
            texto,
            &mut gerar,
            &executar,
        );
        (codigo, String::from_utf8_lossy(&saida).into_owned())
    }

    #[test]
    fn pedido_mostra_filtro_e_resultado_sem_alterar_buffer() {
        let dir = std::env::temp_dir();
        let (codigo, saida) = rodar_com("dobra\n:q\n", "{\"a\":3}", &dir.join("nao-usado.json"));
        assert_eq!(codigo, 0);
        assert!(saida.contains("filter: .a *= 2"));
        assert!(saida.contains("{\"a\":6}"));
    }

    #[test]
    fn aplicar_encadeia_sobre_o_buffer_novo() {
        let dir = std::env::temp_dir();
        let (_, saida) = rodar_com("dobra\n:a\ndobra\n:q\n", "{\"a\":3}", &dir.join("x.json"));
        assert!(
            saida.contains("{\"a\":12}"),
            "segunda dobra vê o buffer aplicado (6*2)"
        );
    }

    #[test]
    fn sair_com_alteracao_pendente_avisa_e_exige_segundo_q() {
        let dir = std::env::temp_dir().join(format!("jqc-repl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let arq = dir.join("doc.json");
        std::fs::write(&arq, "{\"a\":3}").expect("seed");
        let (codigo, saida) = rodar_com("dobra\n:a\n:q\n:q\n", "{\"a\":3}", &arq);
        assert_eq!(codigo, 0);
        assert!(saida.contains("unsaved changes"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn gravar_no_repl_escreve_com_confirmacao() {
        let dir = std::env::temp_dir().join(format!("jqc-replw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let arq = dir.join("doc.json");
        std::fs::write(&arq, "{\"a\":3}").expect("seed");
        let (codigo, _) = rodar_com("dobra\n:a\n:w\ny\n:q\n", "{\"a\":3}", &arq);
        assert_eq!(codigo, 0);
        let gravado = std::fs::read_to_string(&arq).expect("gravado");
        assert!(gravado.contains("\"a\": 6"));
        assert!(arq.with_extension("json.bak").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fim_de_entrada_encerra_sem_panico() {
        let dir = std::env::temp_dir();
        let (codigo, _) = rodar_com("", "{\"a\":1}", &dir.join("x.json"));
        assert_eq!(codigo, 0, "EOF = sessão encerrada normalmente");
    }

    #[test]
    fn fim_de_entrada_com_buffer_nao_gravado_avisa() {
        let dir = std::env::temp_dir().join(format!("jqc-repl-eof-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let arq = dir.join("doc.json");
        std::fs::write(&arq, "{\"a\":3}").expect("seed");
        // Aplica mas nunca chega em :q — a entrada acaba (EOF) com o buffer
        // alterado e não gravado no disco.
        let (codigo, saida) = rodar_com("dobra\n:a\n", "{\"a\":3}", &arq);
        assert_eq!(codigo, 0);
        assert!(
            saida.contains("unsaved changes discarded"),
            "saida: {saida}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
