curl --tlsv1.2 https://sh.rustup.rs -sSf | sh -s -- --default-toolchain stable -y

if command -v yum; then
    # We must set this option otherwise yum will just continue silently if it can't install one of these
    yum --setopt=skip_missing_names_on_install=False install -y openssl-devel opus
fi
