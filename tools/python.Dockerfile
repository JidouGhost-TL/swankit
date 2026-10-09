FROM rust:1.94.1-bookworm
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3-dev python3-venv \
    && rm -rf /var/lib/apt/lists/*
RUN python3 -m venv /opt/python
ENV PATH="/opt/python/bin:${PATH}" CARGO_BUILD_JOBS=1
RUN python -m pip install --no-cache-dir maturin==1.9.6 pytest==8.4.2 Pillow==11.3.0
WORKDIR /workspace/swankit
