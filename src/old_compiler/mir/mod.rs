pub mod builder;
pub mod declarations;
pub mod expresssions;
pub mod mir;
pub mod statements;

pub use builder::MirBuilder;
pub use mir::{MirBlock, MirFunction, MirInstr, MirProgram};

#[cfg(test)]
mod tests;
