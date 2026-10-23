# The Creusot toolchain the proof in escape/ checks under. Creusot runs on Linux, so on
# another host `run.ps1 -Verify` builds this image once and replays the proof in it, with
# the repository bind-mounted at /src. Versions are pinned: Creusot 0.9.0 with the Why3
# and provers its installer pins, except Z3, taken from Debian (4.8.12) because the
# installer's own Z3 build wants a newer glibc than bookworm ships.
FROM rust:1-bookworm
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      opam git rsync curl unzip bubblewrap m4 pkg-config libgmp-dev zlib1g-dev \
      libgtk-3-dev libgtksourceview-3.0-dev z3 \
 && rm -rf /var/lib/apt/lists/*
RUN opam init --bare --disable-sandboxing -y
RUN git clone --branch v0.9.0 --depth 1 https://github.com/creusot-rs/creusot /opt/creusot \
 && cd /opt/creusot \
 && cargo run --release --bin creusot-install -- --external z3 --provers-parallelism 4
ENV PATH=/root/.local/share/creusot/bin:$PATH
WORKDIR /src/verification/creusot/escape
ENV CARGO_TARGET_DIR=/creusot-target
