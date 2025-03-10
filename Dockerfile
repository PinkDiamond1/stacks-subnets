FROM rust:alpine as build

ARG STACKS_NODE_VERSION="No Version Info"
ARG GIT_BRANCH='No Branch Info'
ARG GIT_COMMIT='No Commit Info'

WORKDIR /src

COPY . .

RUN apk add --no-cache musl-dev openssl-dev

RUN mkdir /out

RUN cd testnet/stacks-node && cargo build --features monitoring_prom,slog_json --release

RUN cp target/release/subnet-node /out

FROM alpine:latest

RUN apk add --no-cache openssl musl

COPY --from=build /out/ /bin/

CMD ["subnet-node", "start"]
