# Fuzzamoto Intermediate Representation

An intermediate representation (IR) for fuzzamoto testcases heavily inspired by
the IR used in Fuzzilli
([paper](https://www.ndss-symposium.org/wp-content/uploads/2023/02/ndss2023_f290_paper.pdf),
[repo](https://github.com/googleprojectzero/fuzzilli)).

## Motivation

At a high level fuzzamoto testcases represent a sequence of actions performed
against one or more target nodes (scenarios), i.e. the following actions (p2p
only):

* Changing time (mock time)
* Establishing new connections
* Sending protocol messages on an established connection

Protocol messages in particular are highly structured, as they are serialized
using a [custom
format](https://github.com/bitcoin/bitcoin/blob/master/src/serialize.h),
contain various cryptographic primitives (hash commitments, signatures,
checksums, ...) and must fullfil various other structural requirements to be
considered valid, such as:

* Block headers must point to an prior block via its hash
* Transaction inputs must point to existing unspent transactions outputs
* `blocktxn` messages are only processed if requested after a prior
  `cmpctblock` message
* ...

Therefore, naively fuzzing a scenario with a byte-array fuzzer, using the
following input format will mostly result in fuzzing the message
(de)serialization code and other easy to reach protocol flows.

```rust
pub enum Action {
    Connect { ... },
    SendMessage { ... },
    SetTime { ... },
}

pub struct TestCase {
    pub actions: Vec<Action>,
}

impl<'a> ScenarioInput<'a> for TestCase {
    fn decode(bytes: &'a [u8]) -> Result<Self> {
        TestCase::deserialize(bytes)
    }
}
```

If we want to fuzz deep logic instead, then we'll need to make input
generation/mutation aware of the structural requirements. This is were a custom
IR of testcases becomes useful.

The IR itself is compiled into the simple `TestCase` format from above, however
for the purpose of input generation/mutation the fuzzer operates on the IR as
it contains/enforces the structural requirements.

## Design

Fuzzamoto IR consists of a sequence of operations that take some input
variables and produce variables as output. The IR uses static single
assignement form (SSA), which means every variable in the IR is defined exactly
once. SSA helps simplify define-use analysis, type inference and code
generation among other things.

In the following example (human readable format), the IR describes the creation
of a transaction that is then send to a node in the test via one of the
existing connections.

```
// Context: nodes=1 connections=8 timestamp=1296688802
v0 <- LoadTxo(8e6ccc132c89ddfb90ce0318dd0066fe0e9bba68eb5218876cb56368b8e67619:0, 2500000000, 00204ae81572f06e1b88fd5ced7a1a000945432e83e1551e6f721ee9c00b8cc33260, , 51)
v1 <- LoadTxVersion(2)
v2 <- LoadLockTime(4294967295)
BeginBuildTx(v1, v2) -> v3
  BeginBuildTxInputs -> v4
    v5 <- LoadSequence(4294967295)
    AddTxInput(v4, v0, v5)
  v6 <- EndBuildTxInputs(v4)
  BeginBuildTxOutputs(v6) -> v7
    v8 <- LoadBytes("9b9b9bb9b0b0ffffffffffffb6b6b6b6b69b9b9b9b9b9b686868ff10ff107fafb0ffff7f9b9b9b9b9bffffff80b3")
    BeginWitnessStack -> v9
    v10 <- EndWitnessStack(v9)
    v11 <- BuildPayToWitnessScriptHash(v8, v10)
    v12 <- LoadAmount(79831088)
    AddTxOutput(v7, v11, v12)
    v13 <- LoadBytes("51")
    BeginWitnessStack -> v14
    v15 <- EndWitnessStack(v14)
    v16 <- BuildPayToWitnessScriptHash(v13, v15)
    v17 <- LoadAmount(97883905)
    AddTxOutput(v7, v11, v17)
    v18 <- LoadBytes("51")
    BeginWitnessStack -> v19
    v20 <- EndWitnessStack(v19)
    v21 <- BuildPayToWitnessScriptHash(v18, v20)
    v22 <- LoadAmount(32474173)
    AddTxOutput(v7, v21, v22)
    v23 <- LoadBytes("51")
    BeginWitnessStack -> v24
    v25 <- EndWitnessStack(v24)
    v26 <- BuildPayToWitnessScriptHash(v23, v25)
    v27 <- LoadAmount(94619006)
    AddTxOutput(v7, v26, v27)
    v28 <- LoadBytes("51")
    BeginWitnessStack -> v29
      v30 <- LoadBytes("fcc697079f9ce5b6")
      AddWitness(v29, v30)
    v31 <- EndWitnessStack(v29)
    v32 <- BuildPayToWitnessScriptHash(v28, v31)
    v33 <- LoadAmount(72942531)
    AddTxOutput(v7, v32, v33)
  v34 <- EndBuildTxOutputs(v7)
v35 <- EndBuildTx(v3, v6, v34)
v36 <- LoadConnection(0)
SendTx(v36, v35)
```

`Load*` operations bring constant data from the fuzzer into the context of the
program. All other operations only take variables as inputs.

### Mutations

The following input mutations are implemented:

* Generating new instructions from scratch into an existing program
  ([generators/](src/generators))
* Mutate an existing instruction to take a different variable of the same type
  as input ([mutators/input.rs](src/mutators/input.rs))
* Mutate the input given to `Load*` instructions
  ([mutators/operation.rs](src/mutators/operation.rs))
* Insert an entire program into another one
  ([mutators/combine.rs](src/mutators/combine.rs))
* Append an entire program to another one
  ([mutators/concat.rs](src/mutators/concat.rs))
