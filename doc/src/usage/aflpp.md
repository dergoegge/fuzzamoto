# Fuzzing with AFL++

*Make sure to understand the [system requirements](./requirements.md) before
running fuzzing campaigns.*

---

All fuzzamoto [scenarios](../design/scenarios.md) can be fuzzed with
[AFL++](https://github.com/AFLplusplus/AFLplusplus)'s nyx mode, except for the
[IR](../design/ir.md) scenario (`scenario-ir`).

The [Dockerfile](https://github.com/dergoegge/fuzzamoto/blob/master/Dockerfile)
at the root of the repository contains an example setup for running fuzzamoto
fuzzing campaigns with AFL++.

Build the container image:
```
docker build -t fuzzamoto .
```

And then create a new container from it:

```
docker run --privileged -it -v $PWD:/fuzzamoto fuzzamoto bash
```

`--privileged` is required to enable the use of kvm by Nyx.

### Example: `http-server`

*All commands in this example are supposed to be run inside the docker
container.*

First, build the scenarios and `fuzzamoto-cli` tool:

```
cd /fuzzamoto
BITCOIND_PATH=/bitcoin/build_fuzz/bin/bitcoind cargo build \
  --package fuzzamoto-scenarios --package fuzzamoto-cli \
  --verbose --features "fuzzamoto/fuzz,fuzzamoto-scenarios/fuzz" --release
```

Then, build the crash handler:
```
clang-19 -fPIC -DENABLE_NYX -D_GNU_SOURCE -DNO_PT_NYX \
    ./fuzzamoto-nyx-sys/src/nyx-crash-handler.c -ldl -I. -shared -o libnyx_crash_handler.so
```

And initialize the nyx share dir:
```
./target/release/fuzzamoto-cli init --sharedir /tmp/fuzzamoto_scenario-http-server \
    --crash-handler /fuzzamoto/libnyx_crash_handler.so \
    --bitcoind /bitcoin/build_fuzz/bin/bitcoind \
    --scenario ./target/release/scenario-http-server \
    --nyx-dir /AFLplusplus/nyx_mode
```

AFL++ can't start from an empty corpus, so unless you already have a seed
corpus available, you'll need to create or find at least one seed input
(ideally this is a useful seed not just "AAA"):

```
mkdir /tmp/in && echo "AAA" > /tmp/in/A
```

Once the seed corpus is ready, you'll be able to start the fuzzing campaign:

```
/AFLplusplus/afl-fuzz -X -i /tmp/in -o /tmp/out -- /tmp/fuzzamoto_scenario-http-server
```

## Multi-core campaigns

Running a multi-core campaign is best practice to make use of all available
cores. This can be done with
[`AFL_Runner`](https://github.com/0xricksanchez/AFL_Runner) (installed in the
[Dockerfile](https://github.com/dergoegge/fuzzamoto/blob/master/Dockerfile)).

### Example: `http-server`

```
aflr run --nyx-mode --afl-binary /AFLplusplus/afl-fuzz --target /tmp/fuzzamoto_scenario-http-server/ \
    --input-dir /tmp/http_in/ --output-dir /tmp/http_out/ \
    --runners 16
```
