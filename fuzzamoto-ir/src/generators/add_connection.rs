use crate::{
    Instruction, Operation, Variable,
    generators::{Generator, GeneratorResult, ProgramBuilder},
};
use rand::{Rng, RngCore};

/// `AddConnectionGenerator` generates programs that create a new connection to a node
#[derive(Default)]
pub struct AddConnectionGenerator;

impl<R: RngCore> Generator<R> for AddConnectionGenerator {
    fn generate(&self, builder: &mut ProgramBuilder, rng: &mut R) -> GeneratorResult {
        let num_connections = if rng.gen_bool(0.7) {
            rng.gen_range(1..=5)
        } else {
            rng.gen_range(6..=20)
        };

        for _ in 0..num_connections {
            let node_var = match builder.get_random_variable(rng, Variable::Node) {
                Some(v) => v,
                None => {
                    if builder.context().num_nodes == 0 {
                        return Err(crate::generators::GeneratorError::InvalidContext(
                            builder.context().clone(),
                        ));
                    }

                    builder
                        .append(Instruction {
                            inputs: vec![],
                            operation: Operation::LoadNode(
                                rng.gen_range(0..builder.context().num_nodes),
                            ),
                        })
                        .expect("Inserting LoadNode should always succeed")
                        .pop()
                        .expect("LoadNode should always produce a var")
                }
            };

            // Create connection type variable
            let conn_type = if rng.gen_bool(0.5) {
                "inbound"
            } else {
                "outbound"
            };
            let conn_type_var = builder
                .append(Instruction {
                    inputs: vec![],
                    operation: Operation::LoadConnectionType(conn_type.to_string()),
                })
                .expect("Inserting LoadConnectionType should always succeed")
                .pop()
                .expect("LoadConnectionType should always produce a var");

            builder
                .append(Instruction {
                    inputs: vec![node_var.index, conn_type_var.index],
                    operation: Operation::AddConnection,
                })
                .expect("Inserting AddConnection should always succeed");
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "AddConnectionGenerator"
    }
}
