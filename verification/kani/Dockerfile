# The Kani toolchain the harnesses in this directory run under. Kani runs on Linux, so
# on another host `run.ps1 -Verify` builds this image once and runs the harnesses in it,
# with the repository bind-mounted at /src. Versions are pinned: a proof is a statement
# about one checker as much as about one tree.
FROM rust:1-bookworm
RUN cargo install --locked kani-verifier --version 0.68.0 && cargo kani setup
WORKDIR /src/verification/kani
ENV CARGO_TARGET_DIR=/kani-target
