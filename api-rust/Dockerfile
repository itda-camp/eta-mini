# delivery-eta Rust 서비스 이미지. 저장소 루트를 빌드 컨텍스트로 쓴다.
#   docker build -f api-rust/Dockerfile -t eta-rust .
#   docker run --rm -p 8082:8080 eta-rust

# ── ONNX Runtime: 버전은 uv.lock 의 onnxruntime 을 따른다 (Python 서빙과 같은 런타임) ──
FROM debian:trixie-slim AS onnxruntime
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl unzip \
    && rm -rf /var/lib/apt/lists/*
COPY uv.lock /tmp/uv.lock
RUN set -eu; \
    ORT_VERSION=$(awk '/^name = "onnxruntime"$/ {found=1; next} found && /^version = / {gsub(/"/, "", $3); print $3; exit}' /tmp/uv.lock); \
    [ -n "$ORT_VERSION" ] || { echo "uv.lock 에서 onnxruntime 버전을 찾지 못했다" >&2; exit 1; }; \
    case "$TARGETARCH" in \
      amd64) ORT_RID=linux-x64 ;; \
      arm64) ORT_RID=linux-arm64 ;; \
      *) echo "지원하지 않는 TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    echo "onnxruntime ${ORT_VERSION} (${ORT_RID})"; \
    # GitHub 릴리스 대신 NuGet 패키지에서 받는다 — 같은 libonnxruntime.so 가 runtimes/<플랫폼>/native/ 에 들어 있다
    curl -fsSL "https://www.nuget.org/api/v2/package/Microsoft.ML.OnnxRuntime/${ORT_VERSION}" -o /tmp/ort.nupkg; \
    mkdir -p /opt/onnxruntime/lib; \
    unzip -j -q /tmp/ort.nupkg "runtimes/${ORT_RID}/native/*" -d /opt/onnxruntime/lib; \
    rm /tmp/ort.nupkg

# ── Rust 빌드: ort 는 load-dynamic 으로 빌드해 ONNX Runtime 을 링크하지도 내려받지도 않는다 ──
FROM rust:1.100-slim-trixie AS build
WORKDIR /src
# 의존성만 먼저 빌드해 소스가 바뀌어도 이 층을 다시 쓴다.
COPY api-rust/Cargo.toml api-rust/Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src
COPY api-rust/src ./src
# 복사한 소스의 mtime 이 더미 빌드보다 오래되면 cargo 가 다시 빌드하지 않으므로 touch 한다.
RUN touch src/main.rs && cargo build --release --locked && cp target/release/eta-rust /eta-rust

# ── 실행 이미지 ──
FROM debian:trixie-slim
RUN useradd --system --no-create-home app
COPY --from=onnxruntime /opt/onnxruntime/lib/ /opt/onnxruntime/lib/
COPY --from=build /eta-rust /usr/local/bin/eta-rust
COPY artifacts/model_sklearn.onnx /app/model_sklearn.onnx
ENV ORT_LIB=/opt/onnxruntime/lib/libonnxruntime.so \
    MODEL_PATH=/app/model_sklearn.onnx \
    PORT=8080
USER app
EXPOSE 8080
CMD ["eta-rust"]
