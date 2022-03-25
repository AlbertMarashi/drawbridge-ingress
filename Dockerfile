# Build the executable
FROM rust:bullseye as builder
WORKDIR /app
RUN rustup default nightly

# Install dependencies
COPY Cargo.toml Cargo.lock ./
COPY letsencrypt ./letsencrypt
COPY congress ./congress
COPY tls_acceptor ./tls_acceptor

WORKDIR /app/drawbridge_ingress

COPY ./drawbridge_ingress/Cargo.toml ./
RUN mkdir src/
RUN echo "fn main() {println!(\"if you see this, the build broke\")}" > src/main.rs

RUN cargo install --path .

# Build the executable using the actual source code
COPY ./drawbridge_ingress/ .
RUN cargo build --release

# Copy the executable and extra files ("static") to an empty Docker image
FROM debian:bullseye

# Install libssl-dev and pkg-config
RUN apt-get update && apt-get install -y libssl-dev pkg-config
COPY --from=builder /app/target/release/ ./ingress

RUN apt install -y ca-certificates
RUN sed -i '/^mozilla\/DST_Root_CA_X3.crt$/ s/^/!/' /etc/ca-certificates.conf
RUN update-ca-certificates

CMD [ "./ingress/drawbridge_ingress" ]