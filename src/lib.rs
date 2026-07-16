//! jqc — pedido em linguagem natural → filtro jq executado, 100% offline.
//! Os módulos são públicos para os testes de integração; as decisões de
//! arquitetura estão em docs/DECISIONS.md.

pub mod executor;
pub mod gravar;
pub mod inferencia;
pub mod modelo;
pub mod prompt;
pub mod repl;
