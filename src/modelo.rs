//! Download e cache do GGUF, pinado por revisão (spec §2): primeiro uso baixa
//! do HF com barra de progresso; depois disso a rede nunca mais é tocada.
//! `--offline` transforma "baixaria" em erro (garantia dura).

use std::path::PathBuf;

pub const REPO: &str = "DominuZ/jq-coder-0.6B";
pub const REVISAO_PINADA: &str = "5f175113a93cdf4b22039b748e15247cdfb08cb1";
pub const ARQUIVO_GGUF: &str = "jq-coder-v13-release-Q8_0.gguf";

#[derive(Debug, thiserror::Error)]
pub enum ErroModelo {
    #[error("could not resolve a cache directory for this OS")]
    SemDirDeCache,
    #[error("--offline was given but the model is not cached at {0}")]
    OfflineSemCache(PathBuf),
    #[error("model download failed: {0}")]
    Download(String),
    #[error("could not write the model to the cache: {0}")]
    Escrita(#[from] std::io::Error),
    #[error(
        "invalid model revision {0:?}: must be non-empty and contain only \
         letters, digits, '.', '_' or '-' (not '.' or '..')"
    )]
    RevisaoInvalida(String),
}

/// A revisão vira componente de caminho do cache (`<cache>/<revisao>/<arquivo>`);
/// sem isto um `--modelo` malicioso poderia escapar do diretório de cache
/// (path traversal via `..` ou separadores).
fn validar_revisao(revisao: &str) -> Result<(), ErroModelo> {
    let valida = !revisao.is_empty()
        && revisao != "."
        && revisao != ".."
        && revisao
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valida {
        Ok(())
    } else {
        Err(ErroModelo::RevisaoInvalida(revisao.to_string()))
    }
}

fn dir_cache() -> Result<PathBuf, ErroModelo> {
    if let Ok(forcado) = std::env::var("JQC_CACHE_DIR") {
        return Ok(PathBuf::from(forcado));
    }
    dirs::cache_dir()
        .map(|base| base.join("jq-coder"))
        .ok_or(ErroModelo::SemDirDeCache)
}

fn caminho_gguf(revisao: &str) -> Result<PathBuf, ErroModelo> {
    Ok(dir_cache()?.join(revisao).join(ARQUIVO_GGUF))
}

/// Garante o GGUF no cache e devolve o caminho. Nunca toca a rede se o
/// arquivo existe; nunca toca a rede com `offline` (falha em vez disso).
pub fn garantir_modelo(revisao: &str, offline: bool) -> Result<PathBuf, ErroModelo> {
    validar_revisao(revisao)?;
    let destino = caminho_gguf(revisao)?;
    if destino.is_file() {
        return Ok(destino);
    }
    if offline {
        return Err(ErroModelo::OfflineSemCache(destino));
    }
    baixar(revisao, &destino)?;
    Ok(destino)
}

fn baixar(revisao: &str, destino: &std::path::Path) -> Result<(), ErroModelo> {
    let url = format!("https://huggingface.co/{REPO}/resolve/{revisao}/{ARQUIVO_GGUF}");
    eprintln!("downloading {ARQUIVO_GGUF} (first run only)…");
    let mut resposta = ureq::get(&url)
        .call()
        .map_err(|erro| ErroModelo::Download(erro.to_string()))?;
    let total: u64 = resposta
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let barra = indicatif::ProgressBar::new(total);
    barra.set_style(
        indicatif::ProgressStyle::with_template("{bar:40} {bytes}/{total_bytes} ({eta})")
            .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar()),
    );

    if let Some(pai) = destino.parent() {
        std::fs::create_dir_all(pai)?;
    }
    // .part.<pid> (não um nome fixo) + rename: dois processos baixando ao
    // mesmo tempo (dois primeiros usos concorrentes) não podem escrever no
    // mesmo arquivo parcial — isso corromperia o cache pra sempre, porque
    // garantir_modelo só confere is_file(). Cada processo tem seu próprio
    // parcial; o rename final é a linha de corte "virou cache válido".
    let parcial = destino.with_extension(format!("gguf.part.{}", std::process::id()));
    let resultado: Result<(), ErroModelo> = (|| {
        let mut arquivo = std::fs::File::create(&parcial)?;
        let mut leitor = resposta.body_mut().as_reader();
        std::io::copy(&mut barra.wrap_read(&mut leitor), &mut arquivo)
            .map_err(|erro| ErroModelo::Download(erro.to_string()))?;
        Ok(())
    })();
    barra.finish_and_clear();
    if let Err(erro) = resultado {
        // Best-effort: só limpamos o NOSSO parcial (nome tem nosso pid); não
        // tentamos coletar lixo de outros processos.
        let _ = std::fs::remove_file(&parcial);
        return Err(erro);
    }
    match std::fs::rename(&parcial, destino) {
        Ok(()) => Ok(()),
        Err(erro) => {
            // Se o destino já existe, outro processo venceu a corrida do
            // download antes de nós — o arquivo dele é válido (só chega
            // aqui depois de um download completo), então usamos o dele e
            // descartamos o nosso parcial. Qualquer outro erro de rename é
            // reportado normalmente.
            if destino.is_file() {
                let _ = std::fs::remove_file(&parcial);
                Ok(())
            } else {
                let _ = std::fs::remove_file(&parcial);
                Err(erro.into())
            }
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    // Os três testes mexem na MESMA env var de processo (JQC_CACHE_DIR); uma
    // trava garante que rodem serializados mesmo se o test harness os
    // escalonar em threads distintas (padrão do cargo test).
    static TRAVA_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn cache_respeita_override_por_env() {
        let _guarda = TRAVA_ENV.lock().expect("trava de teste");
        let dir = tempfile::tempdir().expect("tempdir");
        // SAFETY: teste single-purpose; nenhum outro teste lê JQC_CACHE_DIR
        // concorrentemente com valor diferente deste tempdir.
        unsafe { std::env::set_var("JQC_CACHE_DIR", dir.path()) };
        let caminho = caminho_gguf(REVISAO_PINADA).expect("cache resolvível");
        assert!(caminho.starts_with(dir.path()));
        assert!(caminho.ends_with(format!("{REVISAO_PINADA}/{ARQUIVO_GGUF}")));
        unsafe { std::env::remove_var("JQC_CACHE_DIR") };
    }

    #[test]
    fn offline_sem_cache_falha_sem_tocar_a_rede() {
        let _guarda = TRAVA_ENV.lock().expect("trava de teste");
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe { std::env::set_var("JQC_CACHE_DIR", dir.path()) };
        let resultado = garantir_modelo(REVISAO_PINADA, true);
        assert!(matches!(resultado, Err(ErroModelo::OfflineSemCache(_))));
        unsafe { std::env::remove_var("JQC_CACHE_DIR") };
    }

    #[test]
    fn revisao_com_travessia_de_caminho_e_rejeitada() {
        let _guarda = TRAVA_ENV.lock().expect("trava de teste");
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe { std::env::set_var("JQC_CACHE_DIR", dir.path()) };
        for revisao in ["../etc/passwd", "..", ".", "", "a/b", "a\\b"] {
            let resultado = garantir_modelo(revisao, true);
            assert!(
                matches!(resultado, Err(ErroModelo::RevisaoInvalida(_))),
                "revisão {revisao:?} deveria ser rejeitada, deu {resultado:?}"
            );
        }
        unsafe { std::env::remove_var("JQC_CACHE_DIR") };
    }

    #[test]
    fn arquivo_ja_no_cache_e_devolvido_sem_rede() {
        let _guarda = TRAVA_ENV.lock().expect("trava de teste");
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe { std::env::set_var("JQC_CACHE_DIR", dir.path()) };
        let destino = dir.path().join(REVISAO_PINADA).join(ARQUIVO_GGUF);
        std::fs::create_dir_all(destino.parent().expect("pai")).expect("mkdir");
        std::fs::write(&destino, b"gguf falso").expect("write");
        // offline=true prova que não houve rede: com cache presente, passa.
        let caminho = garantir_modelo(REVISAO_PINADA, true).expect("cache presente");
        assert_eq!(caminho, destino);
        unsafe { std::env::remove_var("JQC_CACHE_DIR") };
    }
}
