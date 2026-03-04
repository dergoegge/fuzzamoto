# CLI Reference

This page covers a few handy `fuzzamoto-cli` workflows. The CLI is built from the
`fuzzamoto-cli` crate in this repository and provides utilities for working with
IR corpora, scenarios, and coverage reports.

## Initialize a campaign with a seed mempool

The `init` subcommand accepts an optional `--seedfile` flag. When provided, the
raw bytes of the seed file are used to create a `mempool.dat` file, which is then 
imported via the `importmempool` RPC **before** the snapshot is taken. This means
every fuzz input will start from a node that already has those transactions in its
mempool, giving the fuzzer richer starting state.

### Using `--seedfile` with `fuzzamoto-cli init`

Pass the path to the seed file with `--seedfile`:

```bash
fuzzamoto-cli init \
  --sharedir /tmp/fuzzamoto_share \
  --crash-handler /path/to/crash_handler.so \
  --bitcoind /path/to/bitcoind \
  --scenario /path/to/scenario-generic \
  --nyx-dir /path/to/nyx \
  --seedfile /tmp/my_seed.dat
```

The generated `fuzz_no_pt.sh` script will automatically download the seed file
into the Nyx VM and pass it to the scenario binary via `--seedfile`. During
scenario initialization the seed file's bytes are written as `mempool.dat` and
imported into the node, then the snapshot is taken with that mempool state so
every test case executes against a node that starts with pre-populated
transactions.

## Generate `ir.context`

The CLI’s `--context` flag expects a context file dumped by the IR scenario. You can produce one outside Nyx as follows:

```bash
cargo build --release -p fuzzamoto-scenarios --bin scenario-ir

DUMP_CONTEXT=/tmp/ir.context \
FUZZAMOTO_INPUT=/dev/null \
RUST_LOG=info \
target/release/scenario-ir /path/to/instrumented/bitcoind
```
After this run, `/tmp/ir.context` contains the serialized `FullProgramContext` used by generators and fuzzing campaigns.

## Generate a sample IR program

Most commands operate on IR programs (`.ir` postcard files). You can generate a
single sample program using the IR generators:

```bash
cargo run -p fuzzamoto-cli -- ir generate \
  --context /path/to/share/dump/ir.context \
  --output /tmp/ir-samples \
  --programs 1 --iterations 8
```

This writes a single `*.ir` file under `/tmp/ir-samples`.

## Inspect an IR program

To print the human-readable SSA form:

```bash
cargo run -p fuzzamoto-cli -- ir print /tmp/ir-samples/<file>.ir
```

Pass `--json` to emit JSON instead.

## Selecting generators

`ir generate` enables a handful of generators by default. You can restrict the
set via `--generators` using the generator names exposed by the IR crate (e.g.,
`AdvanceTimeGenerator`, `HeaderGenerator`, `BlockGenerator`):

```bash
cargo run -p fuzzamoto-cli -- ir generate \
  --context /path/to/share/dump/ir.context \
  --output /tmp/ir-samples \
  --generators AdvanceTimeGenerator,BlockGenerator \
  --programs 16 --iterations 8
```
