curl --tlsv1.2 https://sh.rustup.rs -sSf | sh -s -- --default-toolchain stable -y

if command -v yum; then
    yum install -y openssl-devel opus
fi
