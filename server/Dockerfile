FROM rust:trixie AS build

# create a new empty shell project
RUN USER=root cargo new --bin omniroom
WORKDIR /omniroom

# copy over your manifests
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

# this build step will cache your dependencies
RUN cargo build --release
RUN rm src/*.rs

# copy your source tree
COPY ./src ./src
COPY ./static ./static

# build for release
RUN rm ./target/release/deps/omniroom*
RUN cargo build --release

# our final base
FROM debian:trixie-slim
WORKDIR /app

ENV PORT=
ENV UDP_MUX_PORT=
ENV NAT_IPS=

# copy the build artifact from the build stage
COPY --from=build /omniroom/target/release/omniroom .
COPY --from=build /omniroom/static ./static

# set the startup command to run your binary
CMD ["./omniroom"]
