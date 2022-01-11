# 1: Build the executable
FROM rust:buster as builder
WORKDIR /app
RUN rustup default nightly
# RUN rustup target add --toolchain nightly x86_64-unknown-linux-musl

# 1.1 Install dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir src/
RUN echo "fn main() {println!(\"if you see this, the build broke\")}" > src/main.rs
# RUN cargo install --path . --target x86_64-unknown-linux-musl
RUN cargo install --path .
# 1.2: Build the executable using the actual source code
COPY . .
# ADD --chown=rust:rust . ./
RUN cargo build --release
# RUN cargo build --release --target x86_64-unknown-linux-musl --path .

# 2: Copy the executable and extra files ("static") to an empty Docker image
FROM debian:buster
# install libssl-dev and pkg-config
RUN apt-get update && apt-get install -y libssl-dev pkg-config
COPY --from=builder /app/target/release/ ./ingress
CMD [ "./ingress/drawbridge-ingress" ]