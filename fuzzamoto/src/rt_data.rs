/// The runtime data observed during the course of harness execution
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMetadata {
    // TODO: Add things that are needed
    // for example:
    // txids: Vec<usize>
}
