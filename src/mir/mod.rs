pub mod builder;
pub mod declarations;
pub mod expresssions;
pub mod instructions_http;
pub mod mir;
pub mod statements;

pub use builder::MirBuilder;
pub use instructions_http::*;
pub use mir::{MirBlock, MirFunction, MirInstr, MirProgram};

#[cfg(test)]
mod tests;
