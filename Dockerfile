FROM node:22.12-bookworm-slim AS web-build
WORKDIR /app
COPY package.json package-lock.json ./
COPY apps/web/package.json ./apps/web/package.json
RUN npm ci
COPY apps/web ./apps/web
RUN npm run build

FROM rust:1.89-bookworm AS api-build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY apps/api ./apps/api
RUN cargo build --release --locked -p cook-api

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl nginx \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 cook \
    && mkdir -p /data /run/nginx \
    && chown -R cook:cook /data /run/nginx /var/lib/nginx /var/log/nginx
COPY --from=api-build /app/target/release/cook-api /usr/local/bin/cook-api
COPY --from=web-build /app/apps/web/dist /usr/share/nginx/html
COPY deploy/image-nginx.conf /etc/nginx/nginx.conf
COPY deploy/image-entrypoint.sh /usr/local/bin/cook-entrypoint
RUN chmod 0755 /usr/local/bin/cook-entrypoint
USER cook
ENV API_BIND=127.0.0.1:8081 DATABASE_URL=sqlite:///data/cook.db
EXPOSE 8080
ENTRYPOINT ["cook-entrypoint"]
