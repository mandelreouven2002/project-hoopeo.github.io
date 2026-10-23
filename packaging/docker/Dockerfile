# DEP Engine, built from source (Project Hoopeo).
#
#   docker build -f packaging/docker/Dockerfile -t dep-engine:<v> .
#   docker run -d -p 5433:5433 -v dep-data:/var/lib/dep-engine \
#     -e DEP_ADMIN_PASSWORD=change-me dep-engine:<v>
#
# Connect: dep-cli -h 127.0.0.1 -p 5433 -U admin -d operational --sslmode require

FROM rust:1.97-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY engine engine
COPY packages/dep-cli packages/dep-cli
COPY public/examples public/examples
# The workspace also lists Hoopeo Studio; a stub keeps the manifest valid without building it.
RUN mkdir -p studio/src && printf '[package]\nname = "hoopeo-studio"\nversion = "0.0.0"\nedition = "2021"\npublish = false\n' > studio/Cargo.toml && echo 'fn main() {}' > studio/src/main.rs
RUN cargo build --release -p dep-engine -p dep-cli && strip target/release/dep-engine target/release/dep-cli

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /var/lib/dep-engine --shell /usr/sbin/nologin dep-engine \
 && mkdir -p /var/lib/dep-engine && chown dep-engine:dep-engine /var/lib/dep-engine && chmod 0700 /var/lib/dep-engine
COPY --from=build /src/target/release/dep-engine /src/target/release/dep-cli /usr/local/bin/
COPY public/examples /usr/share/project-hoopeo/examples
COPY NOTICE LICENSING.md public/BRAND.md THIRD-PARTY-NOTICES /usr/share/doc/project-hoopeo/
COPY LICENSES/LicenseRef-Hoopeo-Evaluation.txt LICENSES/LicenseRef-Hoopeo-Commercial.txt /usr/share/doc/project-hoopeo/LICENSES/
USER dep-engine
WORKDIR /var/lib/dep-engine
ENV DEP_LISTEN=0.0.0.0:5433 \
    DEP_DATA_DIR=/var/lib/dep-engine/data \
    DEP_TLS_SELF_SIGNED=1
VOLUME ["/var/lib/dep-engine"]
EXPOSE 5433 8433
LABEL org.opencontainers.image.title="DEP Engine" \
      org.opencontainers.image.description="The Database-Enforced Privacy server (Project Hoopeo)" \
      org.opencontainers.image.vendor="Reuven Mandelovich" \
      org.opencontainers.image.url="https://lp.reuven.click" \
      org.opencontainers.image.source="https://github.com/mandelreouven2002/project-hoopeo.github.io" \
      org.opencontainers.image.licenses="LicenseRef-Hoopeo-Evaluation"
ENTRYPOINT ["/usr/local/bin/dep-engine"]
