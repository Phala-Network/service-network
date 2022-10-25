FROM rust:1.64 as builder

WORKDIR /app
COPY . /app
RUN cargo build --locked --release

FROM debian:11-slim
COPY --from=builder /app/target/release/service_worker /usr/local/bin
COPY --from=builder /app/target/release/service_broker /usr/local/bin

LABEL org.opencontainers.image.source=https://github.com/Phala-Network/service-network
LABEL org.opencontainers.image.description="Phala Service Network"

ARG revision=dev

RUN useradd -m -u 1000 -U -s /bin/sh -d /app app && \
	ldd /usr/local/bin/service_worker && \
    ldd /usr/local/bin/service_broker

USER app
ENV RUST_LOG="service_network=debug,service_broker=debug,service_worker=debug"
ENV PROJECT_GIT_REVISION=$revision
