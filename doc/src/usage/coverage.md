# Coverage Reports

It is possible to generate coverage reports for fuzzamoto scenarios by using the
`fuzzamoto-cli coverage` command. The build steps for doing this are slightly
different than if you were to run `fuzzamoto-cli init`:
- the bitcoind node must be compiled with llvm's [source-based code coverage](https://clang.llvm.org/docs/SourceBasedCodeCoverage.html).
- fuzzamoto's nyx feature should be disabled as coverage tooling does not use snapshots.
- a corpus for the specific scenario is required

The `Dockerfile.coverage` file can be used to run a corpus against a specific scenario.
Both a host directory and a corpus directory must be mounted.

Example:

```
export HOST_OUTPUT_DIR="$(pwd)/coverage-output"
export HOST_CORPUS_DIR="$(pwd)/your-corpus"
export SCENARIO="name"

docker build -t fuzzamoto-coverage -f Dockerfile.coverage .
docker run --privileged -it \
    -v $HOST_OUTPUT_DIR:/mnt/output \
    -v $HOST_CORPUS_DIR:/mnt/corpus \
    fuzzamoto-coverage \
    /fuzzamoto/target/release/scenario-$SCENARIO
```

# Parrallellize coverage measurement
Generating coverage reports is often time-consuming.
In that case, you can benefit from parallelizing the coverage measurement.
To use it, first, you need to build the images defined in both `Dockerfile.coverage` and `Dockerfile.coverage.generic`.

```bash
docker build -f Dockerfile.coverage -t fuzzamoto-coverage .
docker build -f Dockerfile.coverage.generic -t fuzzamoto-coverage-generic .
```

After those images are built, copy the image ID from `fuzzamoto-coverage-generic`.

```bash
docker images | grep fuzzamoto-libafl-generic
```

Lastly, you can run this command to run the `coverage-batch` command for parallelized coverage measurement

```bash
cargo run -p fuzzamoto-cli -- coverage-batch --output ./output --corpus ./corpus --docker-image <image id built from Docker.coverage.generic>
```

This command will use all CPUs available, providing you a significant speedup for covearge measurement.