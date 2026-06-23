# docpipe-server 统一镜像。PDFium + ONNX runtime 随构建产物。
# Builder: 需要预先将 libpdfium.so 放置于 docker/pdfium/ 目录
# （从 pdfium-binaries 对应 pdfium-render 0.9 ABI 的版本下载，见 DEVELOP.md）。
FROM rust:1.85-bookworm AS builder
WORKDIR /build
# 先复制 Cargo 文件利用层缓存（依赖未变时跳过 cargo fetch）
COPY Cargo.toml Cargo.lock ./
COPY crates/docpipe-core/Cargo.toml crates/docpipe-core/
COPY crates/docpipe-server/Cargo.toml crates/docpipe-server/
# 复制 vendor 目录（包含 sqlite-vec patch）
COPY vendor/ vendor/
# 创建空 lib 占位让 cargo fetch 通过依赖解析
RUN mkdir -p crates/docpipe-core/src crates/docpipe-server/src \
    && echo "pub fn _placeholder() {}" > crates/docpipe-core/src/lib.rs \
    && echo "fn main() {}" > crates/docpipe-server/src/main.rs \
    && cargo fetch --locked
# 复制全部源码并正式构建
COPY . .
RUN cargo build --release --bin docpipe-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# PDFium 共享库（构建期 CI 下载 prebuilt，对应 pdfium-render 0.9 ABI，见 DEVELOP.md）。
# 若构建时 docker/pdfium/libpdfium.so 不存在，构建将失败并提示需先下载。
COPY docker/pdfium/libpdfium.so /usr/local/lib/libpdfium.so
RUN test -f /usr/local/lib/libpdfium.so || \
    (echo "ERROR: docker/pdfium/libpdfium.so missing — see DEVELOP.md PDFium section" && exit 1)
ENV PDFIUM_DYNAMIC_LIB_PATH=/usr/local/lib/libpdfium.so
ENV LD_LIBRARY_PATH=/usr/local/lib

COPY --from=builder /build/target/release/docpipe-server /usr/local/bin/docpipe-server

# 数据目录
VOLUME ["/data"]

EXPOSE 8200
ENV BIND_ADDR=0.0.0.0:8200
ENV DATABASE_URL=sqlite:///data/docpipe.db
ENV DOCPIPE_LISTEN=0.0.0.0:8200

CMD ["docpipe-server"]
