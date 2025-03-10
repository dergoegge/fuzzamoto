FROM debian:bookworm

# ------ Build and install dependencies ------

RUN apt-get update

ARG LLVM_V=19

# Make sure we can install the llvm toolchain
RUN apt-get install -y software-properties-common
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
  libzstd-dev

# Install rust and tools
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
RUN rustup install nightly && rustup default nightly

# Clone AFLplusplus and build
ENV LLVM_CONFIG=llvm-config-${LLVM_V}
RUN git clone https://github.com/AFLplusplus/AFLplusplus
RUN cd AFLplusplus && make PERFORMANCE=1 install -j$(nproc --ignore 1)

# Build qemu-nyx and libnyx
RUN cd AFLplusplus/nyx_mode/ && ./build_nyx_support.sh

# Compile nyx htools
RUN cd AFLplusplus/nyx_mode/packer/packer/linux_x86_64-userspace && \
  ./compile_64.sh

# ------ Build Bitcoin Core and the nyx agent ------

# Build Bitcoin Core
ARG RECLONE_BITCOIN=
RUN git clone --branch master https://github.com/bitcoin/bitcoin.git 

ENV CC=$PWD/AFLplusplus/afl-clang-fast
ENV CXX=$PWD/AFLplusplus/afl-clang-fast++
ENV LD=$PWD/AFLplusplus/afl-clang-fast

ENV SOURCES_PATH=/tmp/bitcoin-depends
RUN make -C bitcoin/depends NO_QT=1 NO_BDB=1 NO_ZMQ=1 NO_UPNP=1 NO_NATPMP=1 NO_USDT=1 download-linux SOURCES_PATH=$SOURCES_PATH
# Keep extracted source 
RUN sed -i --regexp-extended '/.*rm -rf .*extract_dir.*/d' ./bitcoin/depends/funcs.mk && \
    make -C ./bitcoin/depends DEBUG=1 NO_QT=1 NO_BDB=1 NO_ZMQ=1 NO_USDT=1 \
      SOURCES_PATH=$SOURCES_PATH \
      AR=llvm-ar-${LLVM_V} NM=llvm-nm-${LLVM_V} RANLIB=llvm-ranlib-${LLVM_V} STRIP=llvm-strip-${LLVM_V} \
      -j$(nproc)

COPY ./target-patches/bitcoin-core-rng.patch bitcoin/
RUN cd bitcoin/ && \
      git apply bitcoin-core-rng.patch

RUN cd bitcoin/ && cmake -B build_fuzz \
      --toolchain ./depends/$(./depends/config.guess)/toolchain.cmake \
      -DENABLE_HARDENING=OFF \
      -DSANITIZERS="address" \
      -DAPPEND_CPPFLAGS="-DFUZZING_BUILD_MODE_UNSAFE_FOR_PRODUCTION"

RUN cmake --build bitcoin/build_fuzz -j$(nproc) --target bitcoind

ENV CC=
ENV CXX=
ENV LD=

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

WORKDIR /fuzzamoto/fuzzamoto-scenarios
COPY ./fuzzamoto-scenarios/Cargo.toml .
COPY ./fuzzamoto-scenarios/bin/ bin/
COPY ./fuzzamoto-scenarios/grammars/ grammars/

WORKDIR /fuzzamoto
COPY ./Cargo.toml .
RUN mkdir .cargo && cargo vendor > .cargo/config

ENV BITCOIND_PATH=/bitcoin/build_fuzz/src/bitcoind
RUN cargo build --workspace --verbose --features nyx,reduced_pow --release

# Build the crash handler
#   -D_GNU_SOURCE & -ldl for `#include <dlfcn.h>`
#   -DNO_PT_NYX for nyx's compile-time instrumentation mode
RUN clang-${LLVM_V} -fPIC -DENABLE_NYX -D_GNU_SOURCE -DNO_PT_NYX \
    ./fuzzamoto-nyx-sys/src/nyx-crash-handler.c -ldl -I. -shared -o libnyx_crash_handler.so

# ------ Create the nyx share dir ------

WORKDIR /

# Create share dir and copy runtime deps into it for each scenario
RUN for scenario in /fuzzamoto/target/release/scenario-*; do \
      if [ -f "$scenario" ] && [ -x "$scenario" ]; then \
      scenario_name=$(basename $scenario); \
      export SCENARIO_NYX_DIR="/tmp/fuzzamoto_${scenario_name}"; \
      /fuzzamoto/target/release/fuzzamoto-cli init \
        --sharedir $SCENARIO_NYX_DIR \
        --crash-handler ./fuzzamoto/libnyx_crash_handler.so \
        --bitcoind bitcoin/build_fuzz/src/bitcoind \
        --scenario $scenario; \
      cp /AFLplusplus/nyx_mode/packer/packer/linux_x86_64-userspace/bin64/* $SCENARIO_NYX_DIR; \
      python3 ./AFLplusplus/nyx_mode/packer/packer/nyx_config_gen.py $SCENARIO_NYX_DIR Kernel -m 4096; \
      fi \
    done
