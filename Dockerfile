# Build stage: compiles the binary with DuckDB bundled (expect 5-10 minutes).
FROM rust:1.89-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

# Runtime stage: a small image with just the binary and your ontology folder.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/stella-ontology /usr/local/bin/stella-ontology
WORKDIR /ontology
# Mount or COPY your ontology folder here. The example is included so the image works out of the box.
COPY examples/supply-chain /ontology
EXPOSE 7070
# serve binds 127.0.0.1 by default; --host 0.0.0.0 is needed inside a container.
ENTRYPOINT ["stella-ontology"]
CMD ["serve", "--dir", "/ontology", "--port", "7070", "--host", "0.0.0.0"]
