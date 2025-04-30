*Work in progress*

# Fuzzamoto: Holistic Fuzzing for Bitcoin Protocol Implementations

Fuzzamoto provides a framework for coverage-guided fuzzing of Bitcoin full node
implementations in a holistic fashion. Instead of the common in-process
`LLVMFuzzerTestOneInput` style tests, testing is performed through external
facing interfaces, such as P2P, RPC, etc ([`Bitcoin
Core`](https://github.com/bitcoin/bitcoin) contributors can think of it as
"Functional Fuzz Tests").

## Design

Snapshot fuzzing lies at the core of Fuzzamoto, to allow for deterministic and
performant fuzzing of one or more full node instances at once. Currently,
only support for snapshot fuzzing with afl++'s [`Nyx`](https://nyx-fuzz.com/) mode
is implemented but future integration with other snapshot fuzzing tools is
possible (e.g. full-system
[`libafl_qemu`](https://github.com/AFLplusplus/LibAFL)).

The rough architecture when fuzzing with fuzzamoto looks as follows:

```
                          Report bug
                               ^
                               |
                              Yes
                               |
         -------------------> Bug? -------------------
         |                                            |
---------|------------ Nyx VM ----------------------  |
|        |                                         |  |
|  -------------                     ------------- |  |
|  | Fuzzamoto | <---p2p/rpc/...---> | Full Node | |  No
|  -------------                     ------------- |  |
|        ^                                         |  |
---------|------------------------------------------  |
         |                                            |
         |                                            |
 Generate testcase < ----------------------------------
```

At the moment, only support for Bitcoin Core as target application is
implemented but the existing abstractions allow for integration with other
projects as well (e.g. [`btcd`](https://github.com/btcsuite/btcd),
[`libbitcoin`](https://github.com/libbitcoin/libbitcoin)).

The full node software under test is extended with a crash handler that reports
application aborts to Nyx (See
[`nyx-crash-handler.c`](fuzzamoto-nyx-sys/src/nyx-crash-handler.c)) and the
harness includes a nyx agent that deals with setup and snapshot creation (See
[`nyx-agent.c`](fuzzamoto-nyx-sys/src/nyx-agent.c)).

## Usage

Actual fuzzing (i.e. input generation) can currently only be done on bare metal
x86-64 systems (limitiation of Nyx). See the [Dockerfile](Dockerfile) for an
example setup.

### Fuzzing with AFL++

Example: fuzzing the http server of Bitcoin Core:

```
$ docker build -t fuzzamoto .
$ docker run --privileged -it fuzzamoto bash
root@...# mkdir /tmp/in && echo "AAA" > /tmp/in/A
root@...# afl-fuzz -X -i /tmp/in -o /tmp/out -- /tmp/fuzzamoto_scenario-http-server
```

#### Multi-core campaigns

Running a multi-core campaign can be done with
[`AFL_Runner`](https://github.com/0xricksanchez/AFL_Runner) (installed in the
[Dockerfile](Dockerfile)).

Example: fuzzing the http server of Bitcoin Core with 16 cores:

```
root@...# aflr run --nyx-mode --target /tmp/fuzzamoto_scenario-http-server/ \
    --input-dir /tmp/http_in/ --output-dir /tmp/http_out/ \
    --runners 16
```

### Fuzzing with fuzzamoto-libafl 

See [Dockerfile.libafl](Dockerfile.libafl) for instructions.

#### Bootstrapping a corpus

`fuzzamoto-libafl` can't start from an empty corpus, at least one valid IR
input needs to exist in the input folder. Inputs can generated from scratch
with `fuzzamoto-cli ir generate`.

```
# Generate a corpus with 32 inputs, each with max. 4 iterations of generation
$ fuzzamoto-cli ir generate --output <output_dir> \
    --iterations 4 --programs 32 \
    --context program.ctx
```

#### Running a campaign

```
$ ./target/release/fuzzamoto-libafl --input /tmp/in --output /tmp/out/ \
    --share /tmp/share-ir/ --buffer-size 4194304
```

#### Upgrading corpora after `fuzzamoto-ir` changes

When new operations are added to `fuzzamoto-ir` the corpus needs to be
converted to a new format, because the default format on disk is `postcard`
which makes inputs with differnt versions of the ir crate incompatible.

The conversion is done with the `fuzzamoto-cli` tool, by converting the current
corpus to json using the old ir crate and then back to ir using the new ir
crate.

Note: if operations themselves were changed (e.g. number of inputs or input
types), then the json conversion will likely not work. In this case a custom
conversion will needs to be written or the corpus may also be re-generated.

```
# Convert the current corpus to json using the old ir crate
$ fuzzamoto-cli ir convert --from postcard --to json <input_corpus> <output_corpus_json>
# Convert back to ir using the new ir crate
$ fuzzamoto-cli ir convert --from json --to postcard <output_corpus_json> <output_corpus>
```

### Reproducing testcases

Crashing inputs or other solutions can be reproduced on any architecture with
something similar to the following:

```
$ # Rebuild fuzzamoto without nyx feature
$ cargo build --release --features inherit_stdout --workspace
$ cat ./testcase.dat | ./target/release/fuzzamoto_scenario-http-server ./bitcoind
```

`--features inherit_stdout` is used to inherit stdout from the target
application, such that any logs, stack traces, etc. are printed to the
terminal.

### Custom target patches

Certain targets require custom patches for effective fuzzing and testcase
reproduction. These can be found in the [`target-patches`](target-patches)
directory.

Maintaining external patches should be avoided if possible, as it has several
downsides:

* They might become outdated and require rebase
* They might not apply to a PR we would like to fuzz, in which case the patch
  needs to be adjusted just for the PR
* Testcases might not reproduce without the patches and it is on the user to
  make sure all patches were applied correctly

If a patch is necessary, then landing it in the target application is preferred
but in the case of a fuzz blocker (e.g. checksum check in the target) the best
solution is to make the harness/test produce valid inputs (if possible).

Current patches:

- `bitcoin-core-rng.patch`: Attempts to make Bitcoin Core's RNG deterministic
- `bitcoin-core-aggressive-rng.patch`: Same as `bitcoin-core-rng.patch` but
  more aggressive

## Bugs found by Fuzzamoto

| Project                                            | Bug                                             | Scenario           | Security |
| :------------------------------------------------- | :---------------------------------------------- | :----------------- | :------- |
| [Bitcoin Core](https://github.com/bitcoin/bitcoin) | https://github.com/bitcoin/bitcoin/issues/32111 | `wallet-migration` | ❌       |
| [Bitcoin Core](https://github.com/bitcoin/bitcoin) | https://github.com/bitcoin/bitcoin/issues/32112 | `wallet-migration` | ❌       |
| [Bitcoin Core](https://github.com/bitcoin/bitcoin) | https://github.com/bitcoin/bitcoin/issues/32173 | `rpc-generic`      | ❌       |

