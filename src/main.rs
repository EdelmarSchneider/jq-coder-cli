//! Fluxo do comando (spec §2 e §5). Códigos de saída: 0 sucesso; 1 erro do
//! modelo (sem filtro / executor rejeitou / timeout); 2 erro de ambiente.
//! Paridade de UX com a CLI Python: `filtro: <programa>` no stderr, resultado
//! puro no stdout (pipeável).

use std::io::Read;
use std::path::PathBuf;

use clap::Parser;

use jqc::{executor, inferencia, modelo, prompt};

/// Translate a natural-language request into a jq filter and run it — offline.
#[derive(Parser)]
#[command(name = "jqc", version)]
struct Args {
    /// e.g. "get the id of every order" (English and Brazilian Portuguese)
    pedido: String,
    /// JSON file to query ("-" reads stdin)
    arquivo: String,
    /// Print only the generated filter, without running it
    #[arg(long = "so-filtro")]
    so_filtro: bool,
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
    // 1. Documento primeiro: erro de arquivo não pode custar 640 MB de download.
    let documento = match ler_documento(&args.arquivo) {
        Ok(texto) => texto,
        Err(erro) => {
            eprintln!("could not read {}: {erro}", args.arquivo);
            return 2;
        }
    };
    let valor: serde_json::Value = match serde_json::from_str(&documento) {
        Ok(v) => v,
        Err(erro) => {
            eprintln!("invalid JSON in {}: {erro}", args.arquivo);
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
            println!("{saida}");
            0
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

fn ler_documento(origem: &str) -> std::io::Result<String> {
    if origem == "-" {
        let mut texto = String::new();
        std::io::stdin().read_to_string(&mut texto)?;
        return Ok(texto);
    }
    // ler como bytes + from_utf8_lossy corromperia silenciosamente; erro é melhor
    std::fs::read_to_string(PathBuf::from(origem))
}
