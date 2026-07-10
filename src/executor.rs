//! Executor de filtros: jaq embutido como biblioteca (ADR-001).
//!
//! A saída é canonicalizada como `jq -cS`: compacta, chaves ordenadas, um
//! valor por linha, sem `\n` final.
//!
//! Adaptação em relação ao design original: jaq-json 2.x não expõe conversão
//! `Val ⇄ serde_json::Value` nem `Serialize` para `Val` (a feature `serde` só
//! traz `Deserialize`; `serde_json` é apenas dev-dependency do próprio
//! jaq-json). A premissa "ordenação vem de graça do serde_json::Map =
//! BTreeMap" não se sustenta porque nunca materializamos um
//! `serde_json::Value` de saída — `Val::Obj` é um `indexmap::IndexMap`
//! (ordem de inserção). Em vez disso, a ordenação vem do escritor nativo do
//! jaq-json: `jaq_json::write::Pp { sort_keys: true, .. }` ordena as chaves
//! na hora de serializar, que é a mesma abordagem do `jq -S` (reordenar na
//! impressão, não na estrutura). A entrada continua passando por
//! `serde_json::from_str` (via `Deserialize` de `Val`) para manter o
//! contrato de erro `ErroExecutor::Json(serde_json::Error)`.
//!
//! Nota: os builtins `input`/`inputs` estão registrados mas não funcionam —
//! este executor é de documento único por design (`data::JustLut`, sem
//! stream de entradas).

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use jaq_core::load::{Arena, File, Loader};
use jaq_core::{Ctx, Vars, data};
use jaq_json::{Val, write};
use wait_timeout::ChildExt;

#[derive(Debug, thiserror::Error)]
pub enum ErroExecutor {
    #[error("invalid JSON input: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the model generated a filter jq rejected: {0}")]
    Compilacao(String),
    #[error("the filter failed at runtime: {0}")]
    Execucao(String),
    #[error("the filter timed out after {0} s")]
    Timeout(u64),
    #[error("could not run the filter subprocess: {0}")]
    Subprocesso(String),
}

/// Espelho do TIMEOUT_PADRAO_S do repo JQ.
pub const TIMEOUT_PADRAO_S: u64 = 5;

/// Executa `filtro` sobre `documento` IN-PROCESS — não é matável; só o
/// subcomando oculto `__filtro` (Task 7) pode chamar isto no caminho do
/// usuário. Todo mundo mais usa `executar_com_timeout`.
pub fn executar(filtro: &str, documento: &str) -> Result<String, ErroExecutor> {
    let entrada: Val = serde_json::from_str(documento)?;

    let programa = File {
        code: filtro,
        path: (),
    };
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modulos = loader
        .load(&arena, programa)
        .map_err(|erros| ErroExecutor::Compilacao(formatar_erros_de_load(&erros)))?;

    let funs = jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());
    let filtro_compilado = jaq_core::Compiler::default()
        .with_funs(funs)
        .compile(modulos)
        .map_err(|erros| ErroExecutor::Compilacao(formatar_erros_de_compile(&erros)))?;

    let ctx = Ctx::<data::JustLut<Val>>::new(&filtro_compilado.lut, Vars::new([]));
    let pp = write::Pp {
        sort_keys: true,
        ..Default::default()
    };

    let mut linhas = Vec::new();
    for resultado in filtro_compilado.id.run((ctx, entrada)) {
        let val = resultado.map_err(|erro| ErroExecutor::Execucao(format!("{erro:?}")))?;
        let mut bytes = Vec::new();
        write::write(&mut bytes, &pp, 0, &val)
            .map_err(|erro| ErroExecutor::Execucao(erro.to_string()))?;
        linhas.push(String::from_utf8_lossy(&bytes).into_owned());
    }
    Ok(linhas.join("\n"))
}

/// O lado FILHO da auto-reinvocação: lê o documento do stdin, executa
/// in-process e conversa com o pai por stdout/stderr + código de saída.
pub fn rodar_modo_filtro(filtro: &str) -> i32 {
    let mut documento = String::new();
    if let Err(erro) = std::io::stdin().read_to_string(&mut documento) {
        eprintln!("could not read stdin: {erro}");
        return 1;
    }
    match executar(filtro, &documento) {
        Ok(saida) => {
            println!("{saida}");
            0
        }
        Err(erro) => {
            eprintln!("{erro}");
            1
        }
    }
}

/// O lado PAI: re-invoca `exe __filtro <filtro>` com o documento no stdin e
/// mata o filho se o timeout estourar. Leitura de stdout/stderr e escrita do
/// stdin acontecem em threads para não travar em pipes cheios.
pub fn executar_com_timeout(
    exe: &Path,
    filtro: &str,
    documento: &str,
    timeout_s: u64,
) -> Result<String, ErroExecutor> {
    let mut filho = Command::new(exe)
        .args(["__filtro", filtro])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|erro| ErroExecutor::Subprocesso(erro.to_string()))?;

    let mut stdin = filho
        .stdin
        .take()
        .ok_or_else(|| ErroExecutor::Subprocesso("no stdin".into()))?;
    let doc = documento.to_string();
    let escritor = std::thread::spawn(move || {
        let _ = stdin.write_all(doc.as_bytes()); // filho pode morrer antes: ok
    });
    let mut stdout = filho
        .stdout
        .take()
        .ok_or_else(|| ErroExecutor::Subprocesso("no stdout".into()))?;
    let leitor_out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let mut stderr = filho
        .stderr
        .take()
        .ok_or_else(|| ErroExecutor::Subprocesso("no stderr".into()))?;
    let leitor_err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let status = filho
        .wait_timeout(Duration::from_secs(timeout_s))
        .map_err(|erro| ErroExecutor::Subprocesso(erro.to_string()))?;
    let Some(status) = status else {
        let _ = filho.kill();
        let _ = filho.wait();
        let _ = escritor.join();
        let _ = leitor_out.join();
        let _ = leitor_err.join();
        return Err(ErroExecutor::Timeout(timeout_s));
    };
    let _ = escritor.join();
    let saida = leitor_out.join().unwrap_or_default();
    let erro = leitor_err.join().unwrap_or_default();
    if status.success() {
        Ok(saida.strip_suffix('\n').unwrap_or(&saida).to_string())
    } else {
        Err(ErroExecutor::Execucao(erro.trim().to_string()))
    }
}

/// Junta os erros de carregamento (léxico/sintático/módulos) numa mensagem curta.
fn formatar_erros_de_load(erros: &jaq_core::load::Errors<&str, ()>) -> String {
    erros
        .iter()
        .map(|(_, erro)| format!("{erro:?}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Junta os erros de compilação (símbolos indefinidos) numa mensagem curta.
fn formatar_erros_de_compile(erros: &jaq_core::compile::Errors<&str, ()>) -> String {
    erros
        .iter()
        .flat_map(|(_, es)| es.iter())
        .map(|(nome, indefinido)| format!("undefined {} '{nome}'", indefinido.as_str()))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn identidade_compacta() {
        assert_eq!(
            executar(".", "{\"a\": 1}").expect("filtro válido"),
            "{\"a\":1}"
        );
    }

    #[test]
    fn chaves_saem_ordenadas_como_jq_s() {
        // A ordenação vem de `jaq_json::write::Pp { sort_keys: true }` no
        // caminho de escrita (Val::Obj é IndexMap, ordem de inserção). Se
        // este teste quebrar, o suspeito é o write path do jaq-json — não
        // features do serde_json.
        assert_eq!(
            executar(".", "{\"b\": 2, \"a\": {\"d\": 4, \"c\": 3}}").expect("filtro válido"),
            "{\"a\":{\"c\":3,\"d\":4},\"b\":2}"
        );
    }

    #[test]
    fn stream_vira_linhas() {
        assert_eq!(
            executar(".[]", "[1,2,3]").expect("filtro válido"),
            "1\n2\n3"
        );
    }

    #[test]
    fn saida_vazia_e_string_vazia() {
        assert_eq!(executar("empty", "null").expect("filtro válido"), "");
    }

    #[test]
    fn filtro_invalido_e_erro_de_compilacao() {
        assert!(matches!(
            executar(".[ |", "null"),
            Err(ErroExecutor::Compilacao(_))
        ));
    }

    #[test]
    fn erro_de_execucao_e_reportado() {
        // indexar número com string é erro de runtime no jq
        assert!(matches!(
            executar(".foo", "42"),
            Err(ErroExecutor::Execucao(_))
        ));
    }

    #[test]
    fn documento_invalido_e_erro_json() {
        assert!(matches!(
            executar(".", "{nao é json"),
            Err(ErroExecutor::Json(_))
        ));
    }

    #[test]
    fn limitacao_conhecida_join_com_null() {
        // ADR-001, consequência 3: jaq imprime "null" onde o jq põe "" —
        // este teste DOCUMENTA a divergência aceita; se o jaq alinhar com o
        // jq um dia, o teste avisa para re-rodar o spike.
        assert_eq!(
            executar("join(\",\")", "[\"a\", null, \"b\"]").expect("filtro válido"),
            "\"a,null,b\""
        );
    }
}
