//! Testes do binário via assert_cmd — nada aqui toca rede nem modelo.

use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn subcomando_filtro_executa_e_canonicaliza() {
    Command::cargo_bin("jqc")
        .expect("binário compilado")
        .args(["__filtro", ".b, .a"])
        .write_stdin("{\"a\": 1, \"b\": {\"z\": 1, \"y\": 2}}")
        .assert()
        .success()
        .stdout("{\"y\":2,\"z\":1}\n1\n");
}

#[test]
fn subcomando_filtro_reporta_erro_com_codigo_1() {
    Command::cargo_bin("jqc")
        .expect("binário compilado")
        .args(["__filtro", ".[ |"])
        .write_stdin("null")
        .assert()
        .code(1);
}

#[test]
fn timeout_mata_filtro_infinito() {
    // Regra inegociável do projeto: jaq in-process não é matável; a
    // auto-reinvocação com kill garante que `until(false; .)` não trava a CLI.
    let inicio = std::time::Instant::now();
    let resultado = jqc::executor::executar_com_timeout(
        Path::new(env!("CARGO_BIN_EXE_jqc")),
        "until(false; .)",
        "1",
        2,
    );
    assert!(matches!(
        resultado,
        Err(jqc::executor::ErroExecutor::Timeout(2))
    ));
    assert!(inicio.elapsed().as_secs() < 10, "kill não pode demorar");
}

#[test]
fn timeout_nao_atrapalha_filtro_rapido() {
    let resultado = jqc::executor::executar_com_timeout(
        Path::new(env!("CARGO_BIN_EXE_jqc")),
        ".a",
        "{\"a\": [3, 2, 1]}",
        jqc::executor::TIMEOUT_PADRAO_S,
    );
    assert_eq!(resultado.expect("filtro válido"), "[3,2,1]");
}

#[test]
fn saida_volumosa_atravessa_o_pipe() {
    let resultado = jqc::executor::executar_com_timeout(
        Path::new(env!("CARGO_BIN_EXE_jqc")),
        "range(200000)",
        "null",
        jqc::executor::TIMEOUT_PADRAO_S,
    )
    .expect("filtro válido");
    assert!(resultado.starts_with("0\n1\n"));
    assert!(resultado.ends_with("\n199999"));
}

#[test]
fn arquivo_ilegivel_sai_com_2_sem_baixar_modelo() {
    let cache = tempfile::tempdir().expect("tempdir");
    Command::cargo_bin("jqc")
        .expect("binário compilado")
        // Cache vazio + --offline: se o fluxo tentasse modelo antes do
        // arquivo, o erro seria outro — a ordem "arquivo primeiro" é contrato.
        .env("JQC_CACHE_DIR", cache.path())
        .args(["--offline", "qualquer pedido", "nao-existe.json"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("nao-existe.json"));
}

#[test]
fn json_invalido_sai_com_2() {
    let dir = tempfile::tempdir().expect("tempdir");
    let arquivo = dir.path().join("ruim.json");
    std::fs::write(&arquivo, "{nao é json").expect("write");
    let cache = tempfile::tempdir().expect("tempdir");
    Command::cargo_bin("jqc")
        .expect("binário compilado")
        .env("JQC_CACHE_DIR", cache.path())
        .args(["--offline", "pedido", arquivo.to_str().expect("utf8")])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("invalid JSON"));
}

#[test]
fn offline_sem_cache_sai_com_2() {
    let dir = tempfile::tempdir().expect("tempdir");
    let arquivo = dir.path().join("ok.json");
    std::fs::write(&arquivo, "{\"a\": 1}").expect("write");
    let cache = tempfile::tempdir().expect("tempdir");
    Command::cargo_bin("jqc")
        .expect("binário compilado")
        .env("JQC_CACHE_DIR", cache.path())
        .args(["--offline", "pedido", arquivo.to_str().expect("utf8")])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("--offline"));
}

#[test]
fn help_nao_vaza_o_subcomando_interno() {
    Command::cargo_bin("jqc")
        .expect("binário compilado")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("__filtro").not());
}

// Regressão do Finding 1 da revisão final: std::env::args() entra em pânico
// em argv não-UTF-8 (comportamento documentado da std); args_os() não. Só
// roda em unix porque construir um OsStr não-UTF-8 de propósito depende de
// OsStrExt (unix); compila em qualquer plataforma via cfg(unix), mas Windows
// (esta máquina) não o executa — CI tem runners ubuntu/macos.
#[cfg(unix)]
#[test]
fn argumento_de_arquivo_nao_utf8_nao_causa_panic() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let arquivo = OsStr::from_bytes(b"\xffnao-utf8.json");
    let saida = Command::cargo_bin("jqc")
        .expect("binário compilado")
        .arg("pedido qualquer")
        .arg(arquivo)
        .output()
        .expect("processo executou até o fim");
    assert!(
        !saida.status.success(),
        "argv não-UTF-8 deveria falhar de forma controlada, não ter sucesso"
    );
    let stderr = String::from_utf8_lossy(&saida.stderr);
    assert!(
        !stderr.contains("panicked"),
        "não pode ser um panic do Rust; stderr: {stderr}"
    );
}

#[test]
fn write_e_so_filtro_sao_mutuamente_exclusivos() {
    Command::cargo_bin("jqc")
        .expect("binário compilado")
        .args(["pedido qualquer", "arquivo.json", "--write", "--so-filtro"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("mutually exclusive"));
}

#[test]
fn write_com_stdin_sai_com_2() {
    Command::cargo_bin("jqc")
        .expect("binário compilado")
        .args(["pedido qualquer", "-", "--write"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("stdin has nowhere to write back"));
}

#[test]
fn yes_sem_write_sai_com_2() {
    Command::cargo_bin("jqc")
        .expect("binário compilado")
        .args(["pedido qualquer", "arquivo.json", "--yes"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains(
            "--yes only makes sense with --write",
        ));
}

#[test]
fn write_com_um_so_positional_sai_com_2() {
    Command::cargo_bin("jqc")
        .expect("binário compilado")
        .args(["pedido qualquer", "--write"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains(
            "do not apply to the interactive session",
        ));
}

#[test]
fn revisao_com_caractere_invalido_sai_com_2_sem_baixar() {
    let cache = tempfile::tempdir().expect("tempdir");
    let dir = tempfile::tempdir().expect("tempdir");
    let arquivo = dir.path().join("ok.json");
    std::fs::write(&arquivo, "{\"a\": 1}").expect("write");
    Command::cargo_bin("jqc")
        .expect("binário compilado")
        .env("JQC_CACHE_DIR", cache.path())
        .args([
            "--offline",
            "--modelo",
            "../etc/passwd",
            "pedido",
            arquivo.to_str().expect("utf8"),
        ])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("revision"));
}
