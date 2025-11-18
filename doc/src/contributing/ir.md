# Extending Fuzzamoto IR

This guide explains how to add a new IR primitive end to end. Refer to the [docs](https://dergoegge.github.io/fuzzamoto/design/ir.html) for a high level design overview.

## Model the primitive

- Add new variable kinds in `fuzzamoto-ir/src/variable.rs`.
- Teach `fuzzamoto-ir/src/operation.rs` about the operation: specify its type signature (`get_input_variables`, `get_output_variables`, `get_inner_output_variables`), scope rules (`is_block_begin`, `is_block_end`, `is_matching_block_begin`, `allow_insertion_in_block`), and mutation/minimization behaviour (`is_operation_mutable`, `mutates_nth_input`, `is_noppable`).
- Update `fuzzamoto-ir/src/instruction.rs` so the builder and mutators can spot context changes (`entered_context_after_execution`) and know whether the operation can be mutated or nopped out.

## Snapshot context & scenario

- If the new instruction needs extra snapshot data, extend `FullProgramContext` in `fuzzamoto-ir/src/lib.rs`.
- Populate that data inside the IR scenario by extending the relevant helpers in `fuzzamoto-scenarios/bin/ir.rs` (`build_*`, `dump_context`, etc.).
- Whenever context data changes, re-run `scenario-ir` to refresh `ir.context` for generators and tests.

## Compiler

- Update `fuzzamoto-ir/src/compiler.rs` so the new operations lower into the appropriate `CompiledAction`s. Make use of consensus encoders from `bitcoin` where possible.
- Add focused unit tests in the same file that compile a tiny program and assert on the serialized bytes.

## Generators & mutators

- Implement a generator under `fuzzamoto-ir/src/generators/` that emits the new instructions in a valid SSA context.
- Export it via `generators/mod.rs` and wire it into:
   - `fuzzamoto-libafl/src/instance.rs` so LibAFL schedules it.
   - `fuzzamoto-cli/src/commands/ir.rs` so `ir generate --generators` can use it.
- Update mutators/minimizers if they must recognize the new variable types or the new begin/end blocks. 
- If the primitive benefits from targeted mutations, add a dedicated mutator under `fuzzamoto-ir/src/mutators/`, export it in `mutators/mod.rs`, and register it beside the other mutators in `fuzzamoto-libafl/src/instance.rs`.
