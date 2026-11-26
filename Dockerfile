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
    && useradd --system --uid 10001 --create-home lorefile && mkdir -p /spaces && chown lorefile:lorefile /spaces
COPY --from=build /src/target/release/lorefile /usr/local/bin/lorefile
# The licence and NOTICE travel with every copy (Apache-2.0, section 4).
COPY LICENSE NOTICE /usr/share/doc/lorefile/
USER lorefile
# One sub-folder per space (brand, business unit, client). Mount a persistent
# volume here; the shipped examples are written under /spaces/.lorefile/examples at start.
VOLUME ["/spaces"]
WORKDIR /spaces
EXPOSE 7070
# serve binds 127.0.0.1 by default; --host 0.0.0.0 is needed inside a container.
# The page, the API and MCP over HTTP ask for a login token: pass one with
# -e LOREFILE_TOKEN=..., or the first start makes one, saves it in
# /spaces/.lorefile/auth.toml and prints the login URL in the container's log.
# Add --allow-edit to the command to let the page and agents write.
ENTRYPOINT ["lorefile"]
CMD ["serve", "--dir", "/spaces", "--port", "7070", "--host", "0.0.0.0"]
