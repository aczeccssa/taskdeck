FROM rust:1.87-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release

FROM scratch AS artifact
COPY --from=builder /build/target/release/taskdeck /taskdeck

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl python3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/taskdeck /usr/local/bin/taskdeck

ENV TASKDECK_HOME=/var/lib/taskdeck \
    TASKDECK_ROLE=leader \
    TASKDECK_LEADER_MODE=pure_master \
    TASKDECK_BIND_HOST=0.0.0.0 \
    TASKDECK_WEB_PORT=9837

VOLUME ["/var/lib/taskdeck"]
EXPOSE 9837

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:9837/healthz >/dev/null || exit 1

ENTRYPOINT ["taskdeck"]
CMD ["daemon"]
