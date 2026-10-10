# syntax=docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e
#
# One image for the whole service: the `api`, `worker` and `migrate` binaries
# and the built web app, which the API serves from the same origin
# (DESIGN.md §13.5). The default command is the API; `worker` and `migrate`
# are run by naming them (see docker-compose.yml), as is `replay-deletions`
# after a restore (docs/operations.md, "Replaying deletions"), and `staff`,
# the owner's command that names who reviews reports (docs/operations.md,
# "Reviewing reports"), and `contact-data`, the owner's command that says how
# far contact details are encrypted and re-encrypts them under a new key
# (docs/operations.md, "Contact data key").
#
# Nothing secret is built in. Every setting, the database connection,
# APP_SECRET and CONTACT_DATA_KEY first of all, comes from the environment
# at run time (.env.example lists them).
#
# Which build it is, is built in: the git commit (`--build-arg GIT_SHA=...`,
# or Render's RENDER_GIT_COMMIT) and the build time (`--build-arg
# BUILD_TIME=...`, else the time of the build). The service reports them in
# GET /v1/meta, the X-Yuppers-Version header, its first log line and the
# yuppers_build_info metric, and the web app on its account and staff screens
# (docs/operations.md, "What is deployed"). Nothing runs git here; a build
# without a commit still builds, and says `unknown`.
#
# Every image is pinned by digest, with its tag kept beside it, so a build
# uses exactly the bytes that were reviewed. Dependabot proposes new digests
# weekly (.github/dependabot.yml).

# ---- The web app ------------------------------------------------------------
FROM mirror.gcr.io/library/node:26-bookworm-slim@sha256:86f07bc9c5dce4578cf37e5a418b7bfc7f817cda25cde66e2b66e95ed86c4567 AS web
WORKDIR /src
# The workspace manifests first, so the dependency layer is reused until one
# of them changes.
COPY package.json package-lock.json tsconfig.base.json ./
COPY apps/web/package.json apps/web/
COPY apps/mobile/package.json apps/mobile/
COPY packages/shared/package.json packages/shared/
COPY packages/api-client/package.json packages/api-client/
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund
COPY apps/web apps/web
COPY packages packages
# Declared here, after the dependency layers, so a new commit rebuilds only
# what it has to. vite.config.ts reads them.
ARG GIT_SHA=""
ARG RENDER_GIT_COMMIT=""
RUN npm run build:web

# ---- The service ------------------------------------------------------------
FROM mirror.gcr.io/library/rust:1.99-bookworm@sha256:114c7a4425406451c2866b6aafe69fe29b1b298832db1277d411ac73c82d04d6 AS backend
WORKDIR /src
COPY backend backend
# The build embeds the wording and the list of languages (backend/build.rs).
COPY packages/shared/wording packages/shared/wording
# Read at compile time (backend/src/build_info.rs).
ARG GIT_SHA=""
ARG RENDER_GIT_COMMIT=""
ARG BUILD_TIME=""
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/backend/target \
    export GIT_SHA="${GIT_SHA:-$RENDER_GIT_COMMIT}" \
    && export BUILD_TIME="${BUILD_TIME:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}" \
    && cd backend \
    && cargo build --release --locked --bin api --bin worker --bin migrate --bin replay-deletions --bin staff --bin contact-data \
    && mkdir -p /out \
    && cp target/release/api target/release/worker target/release/migrate target/release/replay-deletions target/release/staff target/release/contact-data /out/

# ---- The image --------------------------------------------------------------
# A libc and CA certificates, no shell and no package manager. The binaries
# bring their own TLS (rustls), so nothing else is needed.
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f
COPY --from=backend /out/api /out/worker /out/migrate /out/replay-deletions /out/staff /out/contact-data /usr/local/bin/
COPY --from=web /src/apps/web/dist /srv/web
# Listen on every interface, since the container's own address is what the
# host maps; serve the web app built above.
ENV BIND_ADDR=0.0.0.0:8080 \
    WEB_DIR=/srv/web \
    RUST_LOG=info
EXPOSE 8080
USER nonroot
CMD ["/usr/local/bin/api"]
