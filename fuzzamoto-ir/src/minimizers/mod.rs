pub mod cutting;
pub mod nopping;

use crate::Program;

pub trait Minimizer: Iterator<Item = Program> {
    fn new(program: Program) -> Self;
    /// Report successful minimization
    fn success(&mut self);
    /// Report failed minimization
    fn failure(&mut self);
}

