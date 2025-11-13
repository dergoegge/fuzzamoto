FROM debian:bookworm AS builder

# ------ Build and install dependencies ------

ARG LLVM_V=19

# Make sure we can install the llvm toolchain
RUN apt-get update && apt-get install -y software-properties-common
RUN apt-key adv --keyserver keyserver.ubuntu.com --recv-keys 15CF4D18AF4F7421
RUN apt-add-repository "deb http://apt.llvm.org/bookworm/ llvm-toolchain-bookworm-${LLVM_V} main"
RUN apt-add-repository "deb-src http://apt.llvm.org/bookworm/ llvm-toolchain-bookworm-${LLVM_V} main"

# Install deps for AFL++, Nyx, Bitcoin Core
RUN apt-get update && apt-get install -y \
  ninja-build \
  libgtk-3-dev \
  pax-utils \
  python3-msgpack \
  python3-jinja2 \
  curl \
  lld-${LLVM_V} \
  llvm-${LLVM_V} \
  llvm-${LLVM_V}-dev \
  clang-${LLVM_V} \
  cpio \
  git \
  build-essential \
  libtool \
  autotools-dev \
  automake \
  cmake \
  pkg-config \
  bsdmainutils \
  openssh-client \
  libcapstone-dev \
  python3 \
  libzstd-dev \
  libssl-dev

# Install rust and tools
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
RUN rustup install nightly && rustup default nightly
RUN cargo install cargo-afl

WORKDIR /
# Clone AFLplusplus and build
ENV LLVM_CONFIG=llvm-config-${LLVM_V}
RUN git clone https://github.com/AFLplusplus/AFLplusplus
RUN cd AFLplusplus && make PERFORMANCE=1 -j$(nproc --ignore 1)

# ------ Build Bitcoin Core and the nyx agent ------

# Build Bitcoin Core
ARG OWNER=bitcoin
ARG REPO=bitcoin
ARG BRANCH=master
ARG PR_NUMBER=

RUN git clone --depth 1 --branch "$BRANCH" "https://github.com/$OWNER/$REPO.git" && \
    if [ -n "$PR_NUMBER" ]; then \
        cd "$REPO" && \
        git fetch --depth=1 origin "pull/$PR_NUMBER/head:pr-$PR_NUMBER" && \
        git checkout "pr-$PR_NUMBER"; \
    fi

ENV CC=/AFLplusplus/afl-clang-fast
ENV CXX=/AFLplusplus/afl-clang-fast++
ENV LD=/AFLplusplus/afl-clang-fast

COPY ./target-patches/bitcoin-core-ir-denylist.txt /denylist.txt
ENV AFL_LLVM_DENYLIST=/denylist.txt

ENV SOURCES_PATH=/tmp/bitcoin-depends
RUN make -C bitcoin/depends NO_QT=1 NO_ZMQ=1 NO_USDT=1 download-linux SOURCES_PATH=$SOURCES_PATH
# Keep extracted source
RUN sed -i --regexp-extended '/.*rm -rf .*extract_dir.*/d' ./bitcoin/depends/funcs.mk && \
    make -C ./bitcoin/depends DEBUG=1 NO_QT=1 NO_ZMQ=1 NO_USDT=1 \
      SOURCES_PATH=$SOURCES_PATH \
      AR=llvm-ar-${LLVM_V} NM=llvm-nm-${LLVM_V} RANLIB=llvm-ranlib-${LLVM_V} STRIP=llvm-strip-${LLVM_V} \
      -j$(nproc)

COPY ./target-patches/bitcoin-core-aggressive-rng.patch bitcoin/
RUN cd bitcoin/ && \
      git apply bitcoin-core-aggressive-rng.patch

RUN cd bitcoin/ && cmake -B build_fuzz \
      --toolchain ./depends/$(./depends/config.guess)/toolchain.cmake \
      -DSANITIZERS="address" \
      -DAPPEND_CPPFLAGS="-DFUZZAMOTO_FUZZING -DFUZZING_BUILD_MODE_UNSAFE_FOR_PRODUCTION -DABORT_ON_FAILED_ASSUME" \
      -DAPPEND_LDFLAGS="-fuse-ld=lld-${LLVM_V}"

RUN cmake --build bitcoin/build_fuzz -j$(nproc) --target bitcoind

# ------ Build fuzzamoto scenarios ------

WORKDIR /fuzzamoto/fuzzamoto-nyx-sys
COPY ./fuzzamoto-nyx-sys/Cargo.toml .
COPY ./fuzzamoto-nyx-sys/src/ src/
COPY ./fuzzamoto-nyx-sys/build.rs .

WORKDIR /fuzzamoto/fuzzamoto
COPY ./fuzzamoto/Cargo.toml .
COPY ./fuzzamoto/src/ src/

WORKDIR /fuzzamoto/fuzzamoto-cli
COPY ./fuzzamoto-cli/Cargo.toml .
COPY ./fuzzamoto-cli/src/ src/

WORKDIR /fuzzamoto/fuzzamoto-ir
COPY ./fuzzamoto-ir/Cargo.toml .
COPY ./fuzzamoto-ir/src/ src/

WORKDIR /fuzzamoto/fuzzamoto-libafl
COPY ./fuzzamoto-libafl/Cargo.toml .
COPY ./fuzzamoto-libafl/src/ src/

WORKDIR /fuzzamoto/fuzzamoto-scenarios
COPY ./fuzzamoto-scenarios/Cargo.toml .
COPY ./fuzzamoto-scenarios/bin/ bin/

WORKDIR /fuzzamoto
COPY ./Cargo.toml .
RUN mkdir .cargo && cargo vendor > .cargo/config

ENV CC=clang-${LLVM_V}
ENV CXX=clang++-${LLVM_V}

# Build only the IR scenario
ARG MODE=fuzz
ENV BITCOIND_PATH=/bitcoin/build_fuzz/bin/bitcoind
RUN cargo build -p fuzzamoto-scenarios --bin scenario-ir --verbose --features "$MODE,fuzzamoto-ir/$MODE,fuzzamoto/$MODE,fuzzamoto/inherit_stdout" --release

# Build the crash handler
RUN clang-${LLVM_V} -fPIC -DENABLE_NYX -D_GNU_SOURCE -DNO_PT_NYX \
    ./fuzzamoto-nyx-sys/src/nyx-crash-handler.c -ldl -I. -shared -o /libnyx_crash_handler.so

# ------ Final runtime stage ------

FROM debian:bookworm-slim

# Install only runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
  libstdc++6 \
  libgcc-s1 \
  iproute2 \
  && rm -rf /var/lib/apt/lists/*

# Copy bitcoind and its dependencies from builder
COPY --from=builder /bitcoin/build_fuzz/bin/bitcoind /bitcoind

# Copy the IR scenario binary
COPY --from=builder /fuzzamoto/target/release/scenario-ir /scenario-ir

# Copy the crash handler
COPY --from=builder /libnyx_crash_handler.so /libnyx_crash_handler.so

# Copy init.sh to root
COPY ./workloads/bitcoin-core-ir/init.sh /init.sh
COPY ./workloads/bitcoin-core-ir/bitcoind.sh /bitcoind.sh
RUN chmod +x /init.sh
RUN chmod +x /bitcoind.sh

WORKDIR /
