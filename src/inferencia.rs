//! Inferência in-process com llama-cpp-2 (bindings estáticos do llama.cpp).
//! Greedy (temp 0) e ~256 tokens de teto: filtros jq são curtos; o formato de
//! saída quem garante é o fine-tune, não o prompt (contrato do repo JQ).

use std::num::NonZeroU32;
use std::path::Path;

use llama_cpp_2::TokenToStringError;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

use crate::prompt::Mensagem;

/// Teto de geração: o maior programa do treino fica bem abaixo disso.
const MAX_TOKENS: usize = 256;
const N_CTX: u32 = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Dispositivo {
    /// GPU se o build tiver vulkan/metal, com fallback automático para CPU.
    Auto,
    /// Força CPU.
    Cpu,
}

#[derive(Debug, thiserror::Error)]
pub enum ErroInferencia {
    #[error("could not initialize the llama.cpp backend: {0}")]
    Backend(String),
    #[error("could not load the model: {0}")]
    Carga(String),
    #[error("generation failed: {0}")]
    Geracao(String),
}

pub struct Motor {
    backend: LlamaBackend,
    modelo: LlamaModel,
}

pub fn carregar(gguf: &Path, dispositivo: Dispositivo) -> Result<Motor, ErroInferencia> {
    // llama.cpp despeja ~30 KB de metadados no stderr a cada carga, enterrando
    // o contrato de UX (`filtro: ...`). Desabilitar tem que vir ANTES do init
    // do backend; Once porque o llama-cpp-2 não suporta reconfigurar (os
    // testes de integração carregam o motor mais de uma vez no processo).
    static SILENCIAR_LOG: std::sync::Once = std::sync::Once::new();
    SILENCIAR_LOG.call_once(|| {
        llama_cpp_2::send_logs_to_tracing(
            llama_cpp_2::LogOptions::default().with_logs_enabled(false),
        );
    });
    let backend = LlamaBackend::init().map_err(|e| ErroInferencia::Backend(e.to_string()))?;
    let camadas_gpu: u32 = match dispositivo {
        Dispositivo::Cpu => 0,
        // Sem feature de GPU o parâmetro é inerte; com ela, offload total.
        Dispositivo::Auto => 999,
    };
    let params = LlamaModelParams::default().with_n_gpu_layers(camadas_gpu);
    let modelo = match LlamaModel::load_from_file(&backend, gguf, &params) {
        Ok(m) => m,
        // Fallback automático (spec §3): GPU indisponível/sem memória → CPU.
        Err(_) if dispositivo == Dispositivo::Auto => {
            let cpu = LlamaModelParams::default().with_n_gpu_layers(0);
            LlamaModel::load_from_file(&backend, gguf, &cpu)
                .map_err(|e| ErroInferencia::Carga(e.to_string()))?
        }
        Err(e) => return Err(ErroInferencia::Carga(e.to_string())),
    };
    Ok(Motor { backend, modelo })
}

impl Motor {
    pub fn gerar(&mut self, mensagens: &[Mensagem]) -> Result<String, ErroInferencia> {
        let erro = |e: String| ErroInferencia::Geracao(e);

        // Template ChatML lido do GGUF — o mesmo caminho do llama-server que
        // serviu o modelo no eval do repo JQ.
        let template = self
            .modelo
            .chat_template(None)
            .map_err(|e| erro(e.to_string()))?;
        let chat: Vec<LlamaChatMessage> = mensagens
            .iter()
            .map(|m| LlamaChatMessage::new(m.role.to_string(), m.content.clone()))
            .collect::<Result<_, _>>()
            .map_err(|e| erro(e.to_string()))?;
        let prompt = self
            .modelo
            .apply_chat_template(&template, &chat, true)
            .map_err(|e| erro(e.to_string()))?;

        let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(N_CTX));
        let mut ctx = self
            .modelo
            .new_context(&self.backend, ctx_params)
            .map_err(|e| erro(e.to_string()))?;

        let tokens = self
            .modelo
            .str_to_token(&prompt, AddBos::Never)
            .map_err(|e| erro(e.to_string()))?;
        let mut batch = LlamaBatch::new(tokens.len().max(1), 1);
        let ultimo = tokens.len() as i32 - 1;
        for (i, token) in tokens.iter().enumerate() {
            batch
                .add(*token, i as i32, &[0], i as i32 == ultimo)
                .map_err(|e| erro(e.to_string()))?;
        }
        ctx.decode(&mut batch).map_err(|e| erro(e.to_string()))?;

        let mut amostrador = LlamaSampler::greedy();
        let mut bytes: Vec<u8> = Vec::new();
        for posicao in (tokens.len() as i32..).take(MAX_TOKENS) {
            let token = amostrador.sample(&ctx, batch.n_tokens() - 1);
            if self.modelo.is_eog_token(token) {
                break;
            }
            // Bytes, não str: um caractere multibyte pode atravessar tokens.
            // `token_to_bytes`/`Special` estão deprecated desde 0.1.0 (docs.rs
            // llama-cpp-2 0.1.151); replicamos aqui o retry por tamanho de
            // buffer que eles faziam por baixo, mas contra o substituto
            // recomendado `token_to_piece_bytes`.
            let pedaco = match self.modelo.token_to_piece_bytes(token, 8, true, None) {
                Err(TokenToStringError::InsufficientBufferSpace(tamanho_necessario)) => {
                    // unsigned_abs() cobre os dois sinais (inclusive i32::MIN)
                    // sem caminho de panic — ao contrário de negar e converter
                    // com try_from/expect.
                    self.modelo.token_to_piece_bytes(
                        token,
                        tamanho_necessario.unsigned_abs() as usize,
                        true,
                        None,
                    )
                }
                outro => outro,
            }
            .map_err(|e| erro(e.to_string()))?;
            bytes.extend(pedaco);
            batch.clear();
            batch
                .add(token, posicao, &[0], true)
                .map_err(|e| erro(e.to_string()))?;
            ctx.decode(&mut batch).map_err(|e| erro(e.to_string()))?;
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}
