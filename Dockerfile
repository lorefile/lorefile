# Build stage: compiles the binary with DuckDB bundled (expect 5-10 minutes).
# examples/ and packs/ are compiled in as the read-only shipped spaces.
FROM rust:1.89-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY examples ./examples
COPY packs ./packs
RUN cargo build --release --locked

# Runtime stage: a small image with just the binary. Your spaces live on a volume.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates git && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home stella && mkdir -p /spaces && chown stella:stella /spaces
COPY --from=build /src/target/release/stella-ontology /usr/local/bin/stella-ontology
USER stella
# One sub-folder per space (brand, business unit, client). Mount a persistent
# volume here; the shipped examples are written under /spaces/.stella/examples at start.
VOLUME ["/spaces"]
WORKDIR /spaces
EXPOSE 7070
# serve binds 127.0.0.1 by default; --host 0.0.0.0 is needed inside a container.
# Add --allow-edit to let the page and agents write; put an authenticating proxy in front first.
ENTRYPOINT ["stella-ontology"]
CMD ["serve", "--dir", "/spaces", "--port", "7070", "--host", "0.0.0.0"]
