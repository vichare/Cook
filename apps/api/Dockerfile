FROM rust:1.88-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY apps/api ./apps/api
RUN cargo build --release --locked -p cook-api

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 cook \
    && mkdir -p /data \
    && chown cook:cook /data
COPY --from=build /app/target/release/cook-api /usr/local/bin/cook-api
USER cook
ENV API_BIND=0.0.0.0:8080 DATABASE_URL=sqlite:///data/cook.db
EXPOSE 8080
ENTRYPOINT ["cook-api"]
