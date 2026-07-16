//! Fluxo do comando (spec §2 e §5). Códigos de saída: 0 sucesso; 1 erro do
//! modelo (sem filtro / executor rejeitou / timeout); 2 erro de ambiente.
//! Paridade de UX com a CLI Python: `filtro: <programa>` no stderr, resultado
//! puro no stdout (pipeável).

use std::io::Read;
use std::path::{Path, PathBuf};

use clap::Parser;

use jqc::{executor, gravar, inferencia, modelo, prompt};

/// Translate a natural-language request into a jq filter and run it — offline.
#[derive(Parser)]
#[command(name = "jqc", version)]
struct Args {
    /// e.g. "get the id of every order" (English and Brazilian Portuguese).
    /// With a single argument that is a file path, starts an interactive session.
    pedido: String,
    /// JSON file to query ("-" reads stdin). Omit to treat PEDIDO as the file
    /// and start the interactive session.
    arquivo: Option<String>,
    /// Print only the generated filter, without running it
    #[arg(long = "so-filtro")]
    so_filtro: bool,
    /// Write the result back into FILE (shows a diff and asks first; keeps FILE.bak)
    #[arg(long, short = 'w')]
    write: bool,
    /// Assume "yes" to the write confirmation (for scripts; only with --write)
    #[arg(long, short = 'y')]
    yes: bool,
    /// Never touch the network (fail if the model is not cached)
    #[arg(long)]
    offline: bool,
    /// Inference device
    #[arg(long, value_enum, default_value = "auto")]
    device: inferencia::Dispositivo,
    /// Hugging Face revision of the model to use
    #[arg(long = "modelo", default_value = modelo::REVISAO_PINADA)]
    revisao: String,
}

fn main() {
    // args_os() nunca entra em pânico (ao contrário de args(), documentado
    // como panicável em argv não-UTF-8); só decodificamos o argumento do
    // filtro, e só depois de confirmar o subcomando oculto por OsStr.
    let argumentos: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if argumentos.len() >= 3 && argumentos[1] == std::ffi::OsStr::new("__filtro") {
        // Um filtro jq é texto por definição; um argv não-UTF-8 aqui não pode
        // ser válido, então falha com o mesmo código de "erro de ambiente"
        // (2) usado no resto do main, em vez de arriscar o panic de decodificar
        // à força.
        std::process::exit(match argumentos[2].to_str() {
            Some(filtro) => executor::rodar_modo_filtro(filtro),
            None => {
                eprintln!("internal filter argument is not valid UTF-8");
                2
            }
        });
    }
    std::process::exit(rodar(Args::parse()));
}

fn rodar(args: Args) -> i32 {
    // 0. Validações de uso: baratas, então vêm antes de qualquer I/O ou download.
    let Some(nome_arquivo) = args.arquivo.clone() else {
        eprintln!("interactive mode lands in the next release; for now pass REQUEST and FILE");
        return 2;
    };
    if args.write && args.so_filtro {
        eprintln!("--write and --so-filtro are mutually exclusive");
        return 2;
    }
    if args.write && nome_arquivo == "-" {
        eprintln!("--write needs a real file (stdin has nowhere to write back)");
        return 2;
    }
    if args.yes && !args.write {
        eprintln!("--yes only makes sense with --write");
        return 2;
    }

    // 1. Documento primeiro: erro de arquivo não pode custar 640 MB de download.
    let documento = match ler_documento(&nome_arquivo) {
        Ok(texto) => texto,
        Err(erro) => {
            eprintln!("could not read {nome_arquivo}: {erro}");
            return 2;
        }
    };
    let valor: serde_json::Value = match serde_json::from_str(&documento) {
        Ok(v) => v,
        Err(erro) => {
            eprintln!("invalid JSON in {nome_arquivo}: {erro}");
            return 2;
        }
    };

    // 2. Modelo (download no primeiro uso; --offline é garantia dura).
    let gguf = match modelo::garantir_modelo(&args.revisao, args.offline) {
        Ok(caminho) => caminho,
        Err(erro) => {
            eprintln!("{erro}");
            return 2;
        }
    };
    let mut motor = match inferencia::carregar(&gguf, args.device) {
        Ok(motor) => motor,
        Err(erro) => {
            eprintln!("{erro}");
            return 2;
        }
    };

    // 3. Prompt = contrato do treino; amostra podada, execução no documento inteiro.
    let amostra = prompt::podar_amostra(&documento, &valor);
    let mensagens = prompt::mensagens_de_inferencia(&args.pedido, &amostra);
    let texto = match motor.gerar(&mensagens) {
        Ok(texto) => texto,
        Err(erro) => {
            eprintln!("{erro}");
            return 1;
        }
    };
    let filtro = prompt::extrair_programa(&texto);
    if filtro.is_empty() {
        eprintln!("the model did not return a filter.");
        return 1;
    }
    eprintln!("filtro: {filtro}");
    if args.so_filtro {
        println!("{filtro}");
        return 0;
    }

    // 4. Execução com timeout SEMPRE (auto-reinvocação; regra 1).
    let exe = match std::env::current_exe() {
        Ok(caminho) => caminho,
        Err(erro) => {
            eprintln!("could not locate my own executable: {erro}");
            return 2;
        }
    };
    match executor::executar_com_timeout(&exe, &filtro, &documento, executor::TIMEOUT_PADRAO_S) {
        Ok(saida) => {
            if !args.write {
                println!("{saida}");
                return 0;
            }
            escrever_no_arquivo(&saida, Path::new(&nome_arquivo), &documento, args.yes)
        }
        Err(erro @ executor::ErroExecutor::Subprocesso(_)) => {
            eprintln!("{erro}");
            2
        }
        Err(erro) => {
            eprintln!("{erro}");
            1
        }
    }
}

/// O gate do spec: valida forma, mostra diff, pergunta (a menos de --yes),
/// grava atômico com .bak. Arquivo intocado em QUALQUER caminho de erro.
fn escrever_no_arquivo(saida: &str, caminho: &Path, documento_atual: &str, yes: bool) -> i32 {
    let doc = match gravar::documento_unico(saida) {
        Ok(doc) => doc,
        Err(erro) => {
            eprintln!("{erro}");
            return 1;
        }
    };
    let proposto = gravar::formatar_para_arquivo(&doc);
    let diff = gravar::diff_resumido(documento_atual, &proposto, 40);
    if diff.is_empty() {
        eprintln!("no changes — file left untouched");
        return 0;
    }
    eprintln!("--- {} (current)", caminho.display());
    eprintln!("+++ proposed");
    eprintln!("{diff}");
    if !yes {
        let stdin = std::io::stdin();
        let mut entrada = stdin.lock();
        let mut stderr = std::io::stderr();
        if !gravar::confirmar(&mut entrada, &mut stderr, "write changes? [y/N] ") {
            eprintln!("aborted — file left untouched");
            return 1;
        }
    }
    match gravar::gravar_atomico(caminho, &proposto) {
        Ok(bak) => {
            eprintln!("written; previous version kept at {}", bak.display());
            0
        }
        Err(erro) => {
            eprintln!("{erro}");
            2
        }
    }
}

fn ler_documento(origem: &str) -> std::io::Result<String> {
    if origem == "-" {
        let mut texto = String::new();
        std::io::stdin().read_to_string(&mut texto)?;
        return Ok(texto);
    }
    // ler como bytes + from_utf8_lossy corromperia silenciosamente; erro é melhor
    std::fs::read_to_string(PathBuf::from(origem))
}
