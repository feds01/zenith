# The Linux environment of `just bench-instructions`: CodSpeed's CLI, which
# installs the valgrind fork that its simulation counts instructions with, and
# cargo-codspeed, which builds the benchmarks for it. The pinned toolchain is
# installed on first use, from rust-toolchain.toml.
FROM rust:1-bookworm

# Keep in step with the runner of CodSpeedHQ/action@v5, which
# .github/workflows/codspeed.yml uses.
ARG CODSPEED_VERSION=5.4.2
# Keep in step with cargo-codspeed in .github/workflows/codspeed.yml.
ARG CARGO_CODSPEED_VERSION=5.0.2

RUN set -eu; \
    name="codspeed-runner-$(uname -m)-unknown-linux-musl"; \
    url="https://github.com/CodSpeedHQ/codspeed/releases/download/v${CODSPEED_VERSION}/${name}.tar.gz"; \
    cd /tmp; \
    curl -fsSLO "$url"; \
    curl -fsSLO "$url.sha256"; \
    echo "$(cut -d ' ' -f 1 "${name}.tar.gz.sha256")  ${name}.tar.gz" | sha256sum -c -; \
    tar -xzf "${name}.tar.gz"; \
    install "${name}/codspeed" /usr/local/bin/codspeed; \
    rm -rf /tmp/*

# This installs the valgrind fork. The walltime executor's setup needs
# privileges that a build doesn't have, and isn't needed.
RUN codspeed setup || true; valgrind --version | grep -q codspeed

RUN cargo install --locked cargo-codspeed@${CARGO_CODSPEED_VERSION}
