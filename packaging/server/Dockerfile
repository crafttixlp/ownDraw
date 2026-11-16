# OwnDraw-Server (mehrstufig). Build-Kontext ist die Wurzel des Repos:
#   ./build-web.sh                                   # baut dist/web/ (die Web-App); MUSS vorher laufen
#   docker build -f packaging/server/Dockerfile -t owndraw-server .
#
# Die Web-App wird hier nicht gebaut, sondern aus dist/web/ kopiert.

FROM rust:1-bookworm AS build
WORKDIR /src
# Weniger Speicherbedarf beim Linken (auch auf kleinen Servern machbar).
ENV CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=4
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p owndraw-server

# Nur das Binary (für packaging/server/release-server.sh --docker: gleiche glibc wie das Laufzeit-Image).
FROM scratch AS binary
COPY --from=build /src/target/release/owndraw-server /owndraw-server

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /app --shell /usr/sbin/nologin owndraw
WORKDIR /app
COPY --from=build /src/target/release/owndraw-server /app/owndraw-server
# Web-App: vorher ./build-web.sh ausführen (erzeugt dist/web/).
COPY dist/web /app/web
COPY packaging/server/entrypoint.sh /app/entrypoint.sh
RUN chmod 755 /app/entrypoint.sh /app/owndraw-server && mkdir -p /app/data && chown owndraw:owndraw /app/data
ENV OWNDRAW_DATA=/app/data \
    OWNDRAW_WEB=/app/web \
    OWNDRAW_BIND=0.0.0.0:8080
VOLUME /app/data
EXPOSE 8080
USER owndraw
HEALTHCHECK --interval=30s --timeout=5s --start-period=40s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8080/api/health || exit 1
ENTRYPOINT ["/app/entrypoint.sh"]
